#!/usr/bin/env python3
# [OPUS-5] Hermetic tests for the per-crate test-PRESENCE gate
# (scripts/coverage-presence.py + bench/coverage-presence.json), issue #5140.
#
# The gate's whole purpose is "a crate cannot silently lose its tests". #5140 found
# the hole underneath it: --check only ever iterated the RECORDED floors, so the 26
# crates that had never been seeded were not passing the gate — they were OUTSIDE it.
# These tests pin both halves of the fix:
#   - the UNTRACKED arm: a crate on disk with no floor entry FAILS --check;
#   - the committed floor file actually covers every crate on disk today;
#   - --seed preserves a hand-written per-crate `note` (so the "why is this floor 0"
#     rationale cannot be erased by the next re-seed);
# plus the pre-existing contracts they must not break (count floor, integration-dir
# presence, ratchet-only-rises).
#
# Hermetic: drives scan/seed/check against a TEMP crates/ tree by pointing the module's
# CRATES_DIR at it — no cargo, no git, no network. The one repo-level test reads the
# committed bench/coverage-presence.json + crates/ listing, both deterministic files.
#
# Run:  python3 scripts/tests/test_coverage_presence.py
# (stdlib only; no pytest required — also discoverable by `pytest`.)

from __future__ import annotations

import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, REPO_ROOT / "scripts" / filename)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


cp = _load("coverage_presence", "coverage-presence.py")


class _Tree:
    """A temp crates/ tree + floor file, with CRATES_DIR pointed at it."""

    def __init__(self, crates: dict[str, dict]):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.crates_dir = root / "crates"
        self.floor = root / "coverage-presence.json"
        for crate, spec in crates.items():
            cdir = self.crates_dir / crate
            (cdir / "src").mkdir(parents=True)
            (cdir / "Cargo.toml").write_text(f'[package]\nname = "{crate}"\n')
            body = "".join(f"#[test]\nfn t{i}() {{}}\n" for i in range(spec.get("tests", 0)))
            (cdir / "src" / "lib.rs").write_text(body)
            if spec.get("integration"):
                (cdir / "tests").mkdir()
                (cdir / "tests" / "it.rs").write_text("")
        self._saved = cp.CRATES_DIR
        cp.CRATES_DIR = str(self.crates_dir)

    def write_floor(self, crates: dict[str, dict]) -> None:
        self.floor.write_text(json.dumps({"_comment": [], "crates": crates}))

    def read_floor(self) -> dict:
        return json.loads(self.floor.read_text())["crates"]

    def check(self) -> tuple[int, str]:
        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = cp.check(str(self.floor))
        return rc, buf.getvalue()

    def seed(self, allow_lower: bool = False) -> tuple[int, str]:
        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = cp.seed(str(self.floor), allow_lower)
        return rc, buf.getvalue()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        cp.CRATES_DIR = self._saved
        self.tmp.cleanup()


class TestUntrackedCrateFails(unittest.TestCase):
    """#5140: a crate on disk with NO floor entry is un-gated, and must RED."""

    def test_untracked_crate_fails_check(self):
        with _Tree({"sparq-a": {"tests": 3}, "sparq-new": {"tests": 26}}) as t:
            t.write_floor({"sparq-a": {"min_tests": 3, "had_integration_dir": False}})
            rc, out = t.check()
            self.assertEqual(rc, 1, out)
            self.assertIn("sparq-new", out)
            self.assertIn("NO presence floor", out)
            # The remediation must be in the failure text — the gate is only useful
            # if the reader knows the one command that clears it.
            self.assertIn("--seed", out)

    def test_untracked_crate_is_not_reported_as_ok(self):
        with _Tree({"sparq-a": {"tests": 3}, "sparq-new": {"tests": 26}}) as t:
            t.write_floor({"sparq-a": {"min_tests": 3, "had_integration_dir": False}})
            _, out = t.check()
            self.assertNotIn("ok   sparq-new", out)

    def test_zero_test_untracked_crate_still_fails(self):
        # A brand-new crate with no tests yet is exactly the case that must not slip
        # in un-gated: it needs an entry so its floor can ratchet up later.
        with _Tree({"sparq-a": {"tests": 1}, "sparq-seam": {"tests": 0}}) as t:
            t.write_floor({"sparq-a": {"min_tests": 1, "had_integration_dir": False}})
            rc, out = t.check()
            self.assertEqual(rc, 1, out)
            self.assertIn("sparq-seam", out)

    def test_fully_tracked_tree_passes(self):
        with _Tree({"sparq-a": {"tests": 3}, "sparq-b": {"tests": 0}}) as t:
            t.write_floor({
                "sparq-a": {"min_tests": 3, "had_integration_dir": False},
                "sparq-b": {"min_tests": 0, "had_integration_dir": False},
            })
            rc, out = t.check()
            self.assertEqual(rc, 0, out)

    def test_seed_then_check_is_clean(self):
        with _Tree({"sparq-a": {"tests": 3}, "sparq-new": {"tests": 26}}) as t:
            t.write_floor({"sparq-a": {"min_tests": 3, "had_integration_dir": False}})
            t.seed()
            rc, out = t.check()
            self.assertEqual(rc, 0, out)


class TestCommittedFloorFileCoversEveryCrate(unittest.TestCase):
    """The #5140 data fix itself: no crate on disk may be missing from the floor file."""

    def test_every_crate_on_disk_has_a_floor_entry(self):
        floors = json.loads((REPO_ROOT / "bench" / "coverage-presence.json").read_text())["crates"]
        crates_dir = REPO_ROOT / "crates"
        on_disk = sorted(
            d.name for d in crates_dir.iterdir()
            if d.is_dir() and (d / "Cargo.toml").exists()
        )
        missing = [c for c in on_disk if c not in floors]
        self.assertEqual(missing, [], f"un-gated crates (run --seed): {missing}")

    def test_no_floor_entry_for_a_crate_that_no_longer_exists(self):
        floors = json.loads((REPO_ROOT / "bench" / "coverage-presence.json").read_text())["crates"]
        crates_dir = REPO_ROOT / "crates"
        stale = [c for c in sorted(floors) if not (crates_dir / c / "Cargo.toml").exists()]
        self.assertEqual(stale, [], f"floors for crates that are gone: {stale}")


class TestNotePreservedAcrossSeed(unittest.TestCase):
    """A hand-written rationale for an unusual floor must survive the next --seed."""

    def test_seed_carries_forward_an_existing_note(self):
        with _Tree({"sparq-seam": {"tests": 0}}) as t:
            t.write_floor({"sparq-seam": {
                "min_tests": 0, "had_integration_dir": False,
                "note": "reserved seam crate: exposes no API yet",
            }})
            t.seed()
            self.assertEqual(
                t.read_floor()["sparq-seam"]["note"],
                "reserved seam crate: exposes no API yet",
            )

    def test_note_survives_a_floor_raise(self):
        with _Tree({"sparq-seam": {"tests": 4}}) as t:
            t.write_floor({"sparq-seam": {
                "min_tests": 0, "had_integration_dir": False, "note": "why zero",
            }})
            t.seed()
            entry = t.read_floor()["sparq-seam"]
            self.assertEqual(entry["min_tests"], 4)
            self.assertEqual(entry["note"], "why zero")

    def test_no_test_crate_note_wins_over_a_stale_hand_note(self):
        with _Tree({"sparq-bench": {"tests": 0}}) as t:
            t.write_floor({"sparq-bench": {
                "min_tests": 0, "had_integration_dir": False, "note": "stale",
            }})
            t.seed()
            self.assertEqual(t.read_floor()["sparq-bench"]["note"],
                             cp.NO_TEST_CRATES["sparq-bench"])

    def test_crate_without_a_note_gets_none(self):
        with _Tree({"sparq-a": {"tests": 2}}) as t:
            t.write_floor({"sparq-a": {"min_tests": 2, "had_integration_dir": False}})
            t.seed()
            self.assertNotIn("note", t.read_floor()["sparq-a"])


class TestPreExistingContracts(unittest.TestCase):
    """The behaviours the #5140 change must not break."""

    def test_tests_below_floor_fails(self):
        with _Tree({"sparq-a": {"tests": 2}}) as t:
            t.write_floor({"sparq-a": {"min_tests": 5, "had_integration_dir": False}})
            rc, out = t.check()
            self.assertEqual(rc, 1, out)
            self.assertIn("2 tests < floor 5", out)

    def test_lost_integration_dir_fails(self):
        with _Tree({"sparq-a": {"tests": 5}}) as t:
            t.write_floor({"sparq-a": {"min_tests": 5, "had_integration_dir": True}})
            rc, out = t.check()
            self.assertEqual(rc, 1, out)
            self.assertIn("lost its tests/ integration dir", out)

    def test_disappeared_crate_fails(self):
        with _Tree({"sparq-a": {"tests": 5}}) as t:
            t.write_floor({
                "sparq-a": {"min_tests": 5, "had_integration_dir": False},
                "sparq-gone": {"min_tests": 5, "had_integration_dir": False},
            })
            rc, out = t.check()
            self.assertEqual(rc, 1, out)
            self.assertIn("DISAPPEARED", out)

    def test_seed_will_not_lower_a_floor(self):
        with _Tree({"sparq-a": {"tests": 2}}) as t:
            t.write_floor({"sparq-a": {"min_tests": 9, "had_integration_dir": False}})
            t.seed()
            self.assertEqual(t.read_floor()["sparq-a"]["min_tests"], 9)

    def test_seed_lowers_only_with_allow_lower(self):
        with _Tree({"sparq-a": {"tests": 2}}) as t:
            t.write_floor({"sparq-a": {"min_tests": 9, "had_integration_dir": False}})
            t.seed(allow_lower=True)
            self.assertEqual(t.read_floor()["sparq-a"]["min_tests"], 2)

    def test_extra_tests_above_floor_pass(self):
        with _Tree({"sparq-a": {"tests": 9}}) as t:
            t.write_floor({"sparq-a": {"min_tests": 5, "had_integration_dir": False}})
            rc, out = t.check()
            self.assertEqual(rc, 0, out)


if __name__ == "__main__":
    unittest.main(verbosity=2)
