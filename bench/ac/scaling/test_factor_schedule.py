#!/usr/bin/env python3
"""Regression tests for the predeclared sensitivity and scenario cells."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "ac_factor_schedule", HERE / "factor_schedule.py"
)
assert SPEC and SPEC.loader
SCHEDULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = SCHEDULE
SPEC.loader.exec_module(SCHEDULE)


class FactorScheduleTests(unittest.TestCase):
    def test_sensitivity_dimensions_are_independent_and_complete(self) -> None:
        cells = SCHEDULE.sensitivity_cells(["social"])
        self.assertEqual(len(cells), 25)
        self.assertEqual(
            {cell.documents for cell in cells if cell.label.startswith("documents-")},
            {1, 8, 32, 128, 512},
        )
        self.assertEqual(
            {cell.triples for cell in cells if cell.label.startswith("triples-")},
            {1, 8, 32, 128},
        )
        placement = [cell for cell in cells if cell.label.startswith("placement-")]
        self.assertEqual(
            {(cell.coverage, cell.depth) for cell in placement},
            {(coverage, depth) for coverage in (0, 100, 250, 1000) for depth in (1, 3, 6)},
        )
        visibility = [cell for cell in cells if cell.label.startswith("visibility-")]
        self.assertEqual({cell.public for cell in visibility}, {10, 100, 500, 1000})
        self.assertTrue(all(cell.public + cell.private + cell.shared == 1000 for cell in cells))

    def test_scenario_anchors_have_declared_counts(self) -> None:
        cells = {cell.label: cell for cell in SCHEDULE.scenario_cells()}
        self.assertEqual(
            (cells["social-count-anchor"].pods, cells["social-count-anchor"].documents),
            (1531, 103),
        )
        self.assertEqual(
            (
                cells["health-compact-anchor"].pods,
                cells["health-compact-anchor"].documents,
                cells["health-compact-anchor"].triples,
            ),
            (256, 1, 128),
        )


if __name__ == "__main__":
    unittest.main()
