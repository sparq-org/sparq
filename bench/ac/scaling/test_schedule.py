#!/usr/bin/env python3
"""Regression tests for the primary Pod-count schedule."""

from __future__ import annotations

import subprocess
import sys
import unittest
from pathlib import Path


HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "schedule.py"


class PrimaryScheduleTests(unittest.TestCase):
    def test_complete_cross_product_has_a_stable_permutation(self) -> None:
        command = [
            sys.executable,
            str(SCRIPT),
            "--lanes",
            "materialized http",
            "--domains",
            "social health",
            "--pods",
            "1 8 64 512 2048",
            "--seed",
            "20260903",
        ]
        first = subprocess.run(command, check=True, capture_output=True, text=True).stdout
        second = subprocess.run(command, check=True, capture_output=True, text=True).stdout
        self.assertEqual(first, second)
        rows = [line.split("\t") for line in first.splitlines()]
        self.assertEqual([int(row[0]) for row in rows], list(range(20)))
        self.assertEqual({int(row[4]) for row in rows}, {20260903})
        self.assertEqual(
            {(row[1], row[2], int(row[3])) for row in rows},
            {
                (lane, domain, pods)
                for lane in ("materialized", "http")
                for domain in ("social", "health")
                for pods in (1, 8, 64, 512, 2048)
            },
        )

    def test_duplicate_dimension_is_rejected(self) -> None:
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--lanes",
                "http http",
                "--domains",
                "social",
                "--pods",
                "1 8",
                "--seed",
                "1",
            ],
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("must not contain duplicates", result.stderr)


if __name__ == "__main__":
    unittest.main()
