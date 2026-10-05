#!/usr/bin/env python3
# sq-toze.29 (GS-5): hermetic tests for scripts/check-vex-deny-drift.py.
#
# Hermetic w.r.t. git/network: imports the drift-check module and drives its pure
# evaluate() + the file parsers against in-tmpdir fixtures. NO subprocess, NO live
# git. A final test runs the parsers against the REAL committed deny.toml + VEX and
# asserts they are in sync (the live invariant GS-5 enforces) — those are committed
# files, not git/network state, so the run stays deterministic.
#
# Run:  python3 scripts/tests/test_vex_deny_drift.py
# (stdlib only; no pytest required — also discoverable by `pytest`.)

from __future__ import annotations

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


drift = _load("check_vex_deny_drift", "check-vex-deny-drift.py")


def _deny(ids: list[str]) -> str:
    """deny.toml fragment with the given ids as structured ignore entries."""
    entries = ",\n".join(
        f'  {{ id = "{i}", reason = "test fixture" }}' for i in ids
    )
    return f"[advisories]\nignore = [\n{entries}\n]\n"


def _vex(ids: list[str]) -> str:
    doc = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "vulnerabilities": [{"id": i, "analysis": {"state": "not_affected"}} for i in ids],
    }
    return json.dumps(doc)


class EvaluatePure(unittest.TestCase):
    def test_in_sync_passes(self):
        ids = {"RUSTSEC-2024-0436", "RUSTSEC-2025-0134"}
        ok, lines = drift.evaluate(ids, set(ids))
        self.assertTrue(ok)
        self.assertTrue(any("in sync" in line for line in lines))

    def test_empty_both_passes(self):
        ok, _ = drift.evaluate(set(), set())
        self.assertTrue(ok)

    def test_id_in_deny_only_fails(self):
        ok, lines = drift.evaluate({"RUSTSEC-2024-0436", "RUSTSEC-2025-0134"}, {"RUSTSEC-2024-0436"})
        self.assertFalse(ok)
        blob = "\n".join(lines)
        self.assertIn("DRIFT", blob)
        self.assertIn("RUSTSEC-2025-0134", blob)
        self.assertIn("MISSING from the VEX", blob)

    def test_id_in_vex_only_fails(self):
        ok, lines = drift.evaluate({"RUSTSEC-2024-0436"}, {"RUSTSEC-2024-0436", "CVE-2025-9999"})
        self.assertFalse(ok)
        blob = "\n".join(lines)
        self.assertIn("DRIFT", blob)
        self.assertIn("CVE-2025-9999", blob)
        self.assertIn("NOT in deny.toml", blob)

    def test_malformed_id_fails(self):
        ok, lines = drift.evaluate({"not-an-advisory"}, {"not-an-advisory"})
        self.assertFalse(ok)
        self.assertIn("Malformed", "\n".join(lines))

    def test_justified_one_sided_passes(self):
        # Inject a justified one-sided entry; it must be tolerated and reported.
        drift.JUSTIFIED_DRIFT["RUSTSEC-2099-0001"] = "test: deliberately deny-only"
        try:
            ok, lines = drift.evaluate(
                {"RUSTSEC-2024-0436", "RUSTSEC-2099-0001"}, {"RUSTSEC-2024-0436"}
            )
            self.assertTrue(ok)
            self.assertIn("justified one-sided", "\n".join(lines))
        finally:
            drift.JUSTIFIED_DRIFT.pop("RUSTSEC-2099-0001", None)


class FileParsers(unittest.TestCase):
    def test_deny_parses_bare_and_structured(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "deny.toml"
            p.write_text(
                '[advisories]\nignore = [ "RUSTSEC-2024-0436", '
                '{ id = "RUSTSEC-2025-0134", reason = "x" } ]\n'
            )
            self.assertEqual(
                drift.deny_ignore_ids(p),
                {"RUSTSEC-2024-0436", "RUSTSEC-2025-0134"},
            )

    def test_deny_reason_text_not_mistaken_for_id(self):
        # A RUSTSEC id mentioned inside the reason prose must NOT be extracted.
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "deny.toml"
            p.write_text(
                '[advisories]\nignore = [ { id = "RUSTSEC-2024-0436", '
                'reason = "see also RUSTSEC-2099-1111 which we do NOT ignore" } ]\n'
            )
            self.assertEqual(drift.deny_ignore_ids(p), {"RUSTSEC-2024-0436"})

    def test_vex_parses(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "vex.json"
            p.write_text(_vex(["RUSTSEC-2024-0436", "RUSTSEC-2025-0134"]))
            self.assertEqual(
                drift.vex_ids(p), {"RUSTSEC-2024-0436", "RUSTSEC-2025-0134"}
            )

    def test_missing_deny_raises_drifterror(self):
        with self.assertRaises(drift.DriftError):
            drift.deny_ignore_ids(Path("/nonexistent/deny.toml"))


class LiveRepoInvariant(unittest.TestCase):
    """The actual GS-5 gate: the committed deny.toml and VEX are in sync."""

    def test_committed_files_in_sync(self):
        d = drift.deny_ignore_ids(REPO_ROOT / "deny.toml")
        v = drift.vex_ids(REPO_ROOT / "supply-chain" / "vex.cdx.json")
        ok, lines = drift.evaluate(d, v)
        self.assertTrue(
            ok,
            "committed deny.toml and supply-chain/vex.cdx.json have drifted:\n"
            + "\n".join(lines),
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
