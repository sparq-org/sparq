#!/usr/bin/env python3
# [OPUS-5] #3767: hermetic tests for scripts/check-npm-advisory-record.py.
#
# Hermetic w.r.t. git/network: drives the pure evaluate()/parse_record()/lock_instances()
# against in-tmpdir fixtures. NO subprocess, NO live git, NO npm. The final test runs the
# real check over the REAL committed record + package-lock.json + package.json and asserts
# they agree — those are committed files, not git/network state, so the run is deterministic.
#
# Every drift class the gate claims to catch has a RED test here: an instance that moved
# version, a copy that appeared, a copy that vanished, a pinning package that moved or
# relaxed its range, and a root override that changed value. Flipping any expected value
# in the check makes at least one of these fail.
#
# Run:  python3 scripts/tests/test_check_npm_advisory_record.py
# (stdlib only; no pytest required — also discoverable by `pytest`.)

from __future__ import annotations

import copy
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, REPO_ROOT / "scripts" / filename)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


chk = _load("check_npm_advisory_record", "check-npm-advisory-record.py")


RECORD = {
    "lock": "package-lock.json",
    "packages": [
        {
            "name": "widget",
            "dependabot_alerts": [1],
            "instances": [
                {
                    "path": "node_modules/widget",
                    "version": "1.1.15",
                    "pinned_by": {
                        "kind": "package",
                        "path": "node_modules/holder",
                        "version": "3.1.5",
                        "range": "^1.1.7",
                    },
                }
            ],
        },
        {
            "name": "gadget",
            "dependabot_alerts": [2],
            "instances": [
                {
                    "path": "node_modules/gadget",
                    "version": "8.5.15",
                    "pinned_by": {"kind": "root_override", "key": "gadget", "value": "8.5.15"},
                }
            ],
        },
        {
            "name": "doodad",
            "dependabot_alerts": [3],
            "instances": [
                {
                    "path": "node_modules/doodad",
                    "version": "0.34.5",
                    "pinned_by": {
                        "kind": "package",
                        "path": "node_modules/framework",
                        "version": "15.5.21",
                        "field": "optionalDependencies",
                        "range": "^0.34.3",
                    },
                }
            ],
        },
    ],
}

LOCK = {
    "node_modules/widget": {"version": "1.1.15"},
    "node_modules/holder": {"version": "3.1.5", "dependencies": {"widget": "^1.1.7"}},
    "node_modules/gadget": {"version": "8.5.15"},
    "node_modules/doodad": {"version": "0.34.5"},
    "node_modules/framework": {
        "version": "15.5.21",
        "optionalDependencies": {"doodad": "^0.34.3"},
    },
}

OVERRIDES = {"gadget": "8.5.15"}


def _md(payload: dict) -> str:
    return (
        "# record\n\nsome prose\n\n<!-- npm-advisory-record:begin -->\n"
        "```json\n" + json.dumps(payload, indent=2) + "\n```\n"
        "<!-- npm-advisory-record:end -->\n"
    )


class EvaluatePure(unittest.TestCase):
    def test_matching_record_passes(self):
        ok, lines = chk.evaluate(RECORD, LOCK, OVERRIDES)
        self.assertTrue(ok, lines)
        self.assertTrue(any("matches the lock" in line for line in lines))

    def test_instance_version_drift_fails(self):
        lock = copy.deepcopy(LOCK)
        lock["node_modules/widget"]["version"] = "1.1.16"
        ok, lines = chk.evaluate(RECORD, lock, OVERRIDES)
        self.assertFalse(ok)
        self.assertTrue(any("'1.1.16'" in line and "record says" in line for line in lines))

    def test_new_unrecorded_instance_fails(self):
        lock = copy.deepcopy(LOCK)
        lock["node_modules/other/node_modules/widget"] = {"version": "5.0.8"}
        ok, lines = chk.evaluate(RECORD, lock, OVERRIDES)
        self.assertFalse(ok)
        self.assertTrue(any("UNRECORDED" in line for line in lines))

    def test_vanished_instance_fails(self):
        lock = copy.deepcopy(LOCK)
        del lock["node_modules/widget"]
        ok, lines = chk.evaluate(RECORD, lock, OVERRIDES)
        self.assertFalse(ok)
        self.assertTrue(any("GONE from the lock" in line for line in lines))

    def test_pinning_package_version_drift_fails(self):
        lock = copy.deepcopy(LOCK)
        lock["node_modules/holder"]["version"] = "3.2.0"
        ok, lines = chk.evaluate(RECORD, lock, OVERRIDES)
        self.assertFalse(ok)
        self.assertTrue(any("pinning package" in line for line in lines))

    def test_pinning_range_drift_fails(self):
        """The crux of the disposition: the pin RELAXING is what unblocks the patch."""
        lock = copy.deepcopy(LOCK)
        lock["node_modules/holder"]["dependencies"]["widget"] = "^5.0.8"
        ok, lines = chk.evaluate(RECORD, lock, OVERRIDES)
        self.assertFalse(ok)
        self.assertTrue(any("'^5.0.8'" in line for line in lines))

    def test_pinning_package_gone_fails(self):
        lock = copy.deepcopy(LOCK)
        del lock["node_modules/holder"]
        ok, lines = chk.evaluate(RECORD, lock, OVERRIDES)
        self.assertFalse(ok)
        self.assertTrue(any("which is GONE" in line for line in lines))

    def test_root_override_bump_fails(self):
        ok, lines = chk.evaluate(RECORD, LOCK, {"gadget": "8.5.18"})
        self.assertFalse(ok)
        self.assertTrue(any("overrides.gadget" in line for line in lines))

    def test_root_override_removed_fails(self):
        ok, lines = chk.evaluate(RECORD, LOCK, {})
        self.assertFalse(ok)
        self.assertTrue(any("overrides.gadget" in line and "None" in line for line in lines))

    def test_optional_dependency_pin_is_read_from_the_recorded_field(self):
        """`sharp` is an OPTIONAL dep of `next` — a dependencies-only lookup would miss it."""
        lock = copy.deepcopy(LOCK)
        lock["node_modules/framework"]["optionalDependencies"]["doodad"] = "^0.35.0"
        ok, lines = chk.evaluate(RECORD, lock, OVERRIDES)
        self.assertFalse(ok)
        self.assertTrue(
            any("optionalDependencies.doodad" in line and "'^0.35.0'" in line for line in lines)
        )

    def test_optional_pin_moved_to_another_field_fails(self):
        """Same range under `dependencies` must not satisfy an `optionalDependencies` pin."""
        lock = copy.deepcopy(LOCK)
        del lock["node_modules/framework"]["optionalDependencies"]
        lock["node_modules/framework"]["dependencies"] = {"doodad": "^0.34.3"}
        ok, lines = chk.evaluate(RECORD, lock, OVERRIDES)
        self.assertFalse(ok)
        self.assertTrue(any("optionalDependencies.doodad = None" in line for line in lines))

    def test_failure_report_says_do_not_relax(self):
        ok, lines = chk.evaluate(RECORD, LOCK, {})
        self.assertFalse(ok)
        self.assertTrue(any("Do NOT relax" in line for line in lines))


class LockInstanceMatching(unittest.TestCase):
    def test_scoped_package_with_same_suffix_is_not_an_instance(self):
        """`node_modules/@tailwindcss/postcss` is NOT a copy of `postcss`."""
        packages = {
            "node_modules/postcss": {"version": "8.5.15"},
            "node_modules/@tailwindcss/postcss": {"version": "4.3.1"},
        }
        self.assertEqual(
            chk.lock_instances(packages, "postcss"), {"node_modules/postcss"}
        )

    def test_nested_and_workspace_copies_are_instances(self):
        packages = {
            "node_modules/x": {},
            "node_modules/a/node_modules/x": {},
            "site/node_modules/x": {},
            "node_modules/xylophone": {},
        }
        self.assertEqual(
            chk.lock_instances(packages, "x"),
            {"node_modules/x", "node_modules/a/node_modules/x", "site/node_modules/x"},
        )


class RecordParsing(unittest.TestCase):
    def _write(self, text: str) -> Path:
        d = Path(self._tmp.name)
        p = d / "npm-advisories.md"
        p.write_text(text, encoding="utf-8")
        return p

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)

    def test_roundtrip(self):
        doc = chk.parse_record(self._write(_md(RECORD)))
        self.assertEqual([p["name"] for p in doc["packages"]], ["widget", "gadget", "doodad"])

    def test_missing_sentinels_is_parse_error(self):
        with self.assertRaises(chk.RecordError):
            chk.parse_record(self._write("# record\n\nno block here\n"))

    def test_missing_fence_is_parse_error(self):
        with self.assertRaises(chk.RecordError):
            chk.parse_record(
                self._write(
                    "<!-- npm-advisory-record:begin -->\nprose only\n"
                    "<!-- npm-advisory-record:end -->\n"
                )
            )

    def test_malformed_json_is_parse_error(self):
        with self.assertRaises(chk.RecordError):
            chk.parse_record(
                self._write(
                    "<!-- npm-advisory-record:begin -->\n```json\n{nope,}\n```\n"
                    "<!-- npm-advisory-record:end -->\n"
                )
            )

    def test_unknown_pin_kind_is_parse_error(self):
        bad = copy.deepcopy(RECORD)
        bad["packages"][0]["instances"][0]["pinned_by"]["kind"] = "vibes"
        with self.assertRaises(chk.RecordError):
            chk.parse_record(self._write(_md(bad)))

    def test_unknown_pin_field_is_parse_error(self):
        """A typo'd field would otherwise compare None to None and pass vacuously."""
        bad = copy.deepcopy(RECORD)
        bad["packages"][0]["instances"][0]["pinned_by"]["field"] = "dependancies"
        with self.assertRaises(chk.RecordError):
            chk.parse_record(self._write(_md(bad)))

    def test_instance_missing_field_is_parse_error(self):
        bad = copy.deepcopy(RECORD)
        del bad["packages"][0]["instances"][0]["version"]
        with self.assertRaises(chk.RecordError):
            chk.parse_record(self._write(_md(bad)))

    def test_missing_record_file_is_parse_error(self):
        with self.assertRaises(chk.RecordError):
            chk.parse_record(Path(self._tmp.name) / "nope.md")


class LiveRepo(unittest.TestCase):
    """The committed record must agree with the committed lock (the live invariant)."""

    def test_committed_record_matches_committed_lock(self):
        record = chk.parse_record(REPO_ROOT / chk.RECORD_PATH)
        packages = chk.load_lock(REPO_ROOT / chk.LOCK_PATH)
        overrides = chk.load_root_overrides(REPO_ROOT / chk.MANIFEST_PATH)
        ok, lines = chk.evaluate(record, packages, overrides)
        self.assertTrue(ok, "\n".join(lines))

    def test_record_covers_the_tracked_packages(self):
        record = chk.parse_record(REPO_ROOT / chk.RECORD_PATH)
        self.assertEqual(
            {p["name"] for p in record["packages"]},
            {"brace-expansion", "postcss", "sharp"},
        )

    def test_main_exits_zero_on_the_live_repo(self):
        self.assertEqual(chk.main([]), 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
