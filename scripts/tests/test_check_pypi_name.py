#!/usr/bin/env python3
# Hermetic tests for scripts/check-pypi-name.py (bead sq-ed5).
#
# Hermetic w.r.t. git/network: imports scripts/check-pypi-name.py and drives its pure
# classify_status() decision table + read_dist_name() reader against the committed
# crates/sparq-py/pyproject.toml. NO network (the live --check path is never exercised) and
# NO subprocess. The one committed-file touch (pyproject.toml) is deterministic.
#
# Run:  python3 scripts/tests/test_check_pypi_name.py
# (stdlib only; no pytest required — also discoverable by `pytest`.)

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(
        name, REPO_ROOT / "scripts" / filename
    )
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


gate = _load("check_pypi_name", "check-pypi-name.py")


class ClassifyStatus(unittest.TestCase):
    """The pure HTTP-status -> verdict decision table. 404 free, 200 taken, anything
    else (incl. None) indeterminate — a non-404 must NEVER read as available."""

    def test_404_available(self):
        self.assertEqual(gate.classify_status(404), gate.AVAILABLE)

    def test_200_taken(self):
        self.assertEqual(gate.classify_status(200), gate.TAKEN)

    def test_403_indeterminate(self):
        self.assertEqual(gate.classify_status(403), gate.INDETERMINATE)

    def test_410_indeterminate(self):
        # 410 Gone is NOT a 404 — must not be treated as available.
        self.assertEqual(gate.classify_status(410), gate.INDETERMINATE)

    def test_500_indeterminate(self):
        self.assertEqual(gate.classify_status(500), gate.INDETERMINATE)

    def test_301_indeterminate(self):
        self.assertEqual(gate.classify_status(301), gate.INDETERMINATE)

    def test_none_transport_error_indeterminate(self):
        # A DNS/connection/timeout (None) is unknown, never "available".
        self.assertEqual(gate.classify_status(None), gate.INDETERMINATE)


class DistNameReader(unittest.TestCase):
    """read_dist_name() pulls the PyPI *distribution* name from the committed
    pyproject.toml. Per §0a / sq-8slf this is `sparq-rdf` (the bare `sparq` is taken).
    Pinning it here means a silent rename back to the taken `sparq` — which would
    reintroduce the exact sq-ed5/sq-8slf clash — fails this test."""

    def test_reads_sparq_rdf(self):
        self.assertEqual(gate.read_dist_name(), "sparq-rdf")

    def test_not_bare_sparq(self):
        # The bare `sparq` distribution name is taken on PyPI; we must not ship as it.
        self.assertNotEqual(gate.read_dist_name(), "sparq")

    def test_pyproject_path_exists(self):
        # Guard against a vacuous reader (e.g. the manifest moving): the file the
        # reader points at must actually exist in the repo.
        self.assertTrue(gate.PYPROJECT.exists(), f"missing {gate.PYPROJECT}")


class VerdictLines(unittest.TestCase):
    """The human-readable verdict rendering carries the right phrasing per verdict."""

    def test_available_mentions_safe(self):
        line = gate._verdict_line("sparq-rdf", gate.AVAILABLE)
        self.assertIn("AVAILABLE", line)
        self.assertIn("safe to publish", line)

    def test_taken_mentions_taken(self):
        line = gate._verdict_line("sparq", gate.TAKEN)
        self.assertIn("TAKEN", line)

    def test_indeterminate_warns(self):
        line = gate._verdict_line("sparq-rdf", gate.INDETERMINATE)
        self.assertIn("do NOT treat as available", line)


class SelfTestEntryPoint(unittest.TestCase):
    """The script's built-in --self-test (the form the ci-scripts job runs) passes on the
    committed tree, AND a bare invocation (no args) defaults to that hermetic self-test
    rather than touching the network."""

    def test_self_test_passes(self):
        self.assertEqual(gate.self_test(), 0)

    def test_bare_main_defaults_to_self_test(self):
        # No --check => hermetic self-test path; returns 0 without any network call.
        self.assertEqual(gate.main([]), 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
