#!/usr/bin/env python3
"""Regression tests for the benchmark resident-memory guard."""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("ac_resource_guard", HERE / "resource_guard.py")
assert SPEC and SPEC.loader
GUARD = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = GUARD
SPEC.loader.exec_module(GUARD)


class ResourceGuardTests(unittest.TestCase):
    def test_empty_directory_is_allowed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            report = GUARD.evaluate(Path(directory), 0.70, 1_000)
        self.assertTrue(report.allowed)
        self.assertIsNone(report.peak_resident_bytes)

    def test_largest_complete_or_partial_construction_is_used(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "one.jsonl").write_text(
                json.dumps({"record_type": "construction", "peak_resident_bytes": 400})
                + "\n",
                encoding="utf-8",
            )
            (root / "two.jsonl.partial").write_text(
                json.dumps({"record_type": "construction", "peak_resident_bytes": 699})
                + "\n",
                encoding="utf-8",
            )
            report = GUARD.evaluate(root, 0.70, 1_000)
        self.assertTrue(report.allowed)
        self.assertEqual(report.peak_resident_bytes, 699)

    def test_observation_at_limit_refuses_next_cell(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "one.jsonl").write_text(
                json.dumps({"record_type": "construction", "peak_resident_bytes": 700})
                + "\n",
                encoding="utf-8",
            )
            report = GUARD.evaluate(root, 0.70, 1_000)
        self.assertFalse(report.allowed)

    def test_invalid_partial_file_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "one.jsonl.partial").write_text("{\n", encoding="utf-8")
            with self.assertRaises(GUARD.GuardError):
                GUARD.evaluate(root, 0.70, 1_000)

    def test_invalid_fraction_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(GUARD.GuardError):
                GUARD.evaluate(Path(directory), 1.0, 1_000)


if __name__ == "__main__":
    unittest.main()
