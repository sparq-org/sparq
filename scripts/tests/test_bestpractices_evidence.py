#!/usr/bin/env python3
# sq-toze.5 (gap GX-4): hermetic tests for scripts/check-bestpractices-evidence.py.
#
# Hermetic w.r.t. git/network: drives the pure evaluate() against in-tmpdir fixtures (no
# subprocess, no live git). A final test runs the checker against the REAL committed
# self-cert (compliance/openssf/best-practices-self-cert.json) and asserts it PASSES — so a
# renamed/deleted evidence file or a bad token in the real file FAILS this suite (and the CI
# lane that runs it).
#
# Run:  python3 scripts/tests/test_bestpractices_evidence.py
# (stdlib only; no pytest required — also discoverable by `pytest`.)

from __future__ import annotations

import importlib.util
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


chk = _load("check_bestpractices_evidence", "check-bestpractices-evidence.py")


def _doc(criteria, *, filed=False, badge_url=None, schema=chk.EXPECTED_SCHEMA):
    project = {"name": "x", "filed": filed}
    if badge_url is not None:
        project["badge_url"] = badge_url
    return {"schema": schema, "project": project, "criteria": criteria}


def _crit(cid="floss_license", *, status="Met", family="basics",
          justification="ok", evidence=("LICENSE",)):
    return {"id": cid, "status": status, "family": family,
            "justification": justification, "evidence": list(evidence)}


class TestEvaluate(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        (self.root / "LICENSE").write_text("MIT", encoding="utf-8")

    def tearDown(self):
        self._tmp.cleanup()

    def test_valid_doc_passes(self):
        ok, lines = chk.evaluate(_doc([_crit()]), self.root)
        self.assertTrue(ok, lines)
        self.assertTrue(lines[0].startswith("OK"), lines)

    def test_missing_evidence_path_fails(self):
        ok, lines = chk.evaluate(_doc([_crit(evidence=("NOPE.md",))]), self.root)
        self.assertFalse(ok)
        self.assertTrue(any("does not resolve" in l for l in lines), lines)

    def test_bad_status_fails(self):
        ok, lines = chk.evaluate(_doc([_crit(status="Probably")]), self.root)
        self.assertFalse(ok)
        self.assertTrue(any("invalid status" in l for l in lines), lines)

    def test_bad_family_fails(self):
        ok, lines = chk.evaluate(_doc([_crit(family="vibes")]), self.root)
        self.assertFalse(ok)
        self.assertTrue(any("invalid family" in l for l in lines), lines)

    def test_duplicate_id_fails(self):
        ok, lines = chk.evaluate(_doc([_crit(), _crit()]), self.root)
        self.assertFalse(ok)
        self.assertTrue(any("duplicate criterion id" in l for l in lines), lines)

    def test_blank_justification_fails(self):
        ok, lines = chk.evaluate(_doc([_crit(justification="  ")]), self.root)
        self.assertFalse(ok)
        self.assertTrue(any("missing/blank justification" in l for l in lines), lines)

    def test_empty_evidence_fails(self):
        ok, lines = chk.evaluate(_doc([_crit(evidence=())]), self.root)
        self.assertFalse(ok)
        self.assertTrue(any("missing/empty evidence" in l for l in lines), lines)

    def test_filed_without_badge_url_fails(self):
        ok, lines = chk.evaluate(_doc([_crit()], filed=True), self.root)
        self.assertFalse(ok)
        self.assertTrue(any("badge_url is missing" in l for l in lines), lines)

    def test_filed_with_badge_url_passes(self):
        ok, lines = chk.evaluate(_doc([_crit()], filed=True, badge_url="https://x"), self.root)
        self.assertTrue(ok, lines)

    def test_wrong_schema_fails(self):
        ok, lines = chk.evaluate(_doc([_crit()], schema="bogus/v9"), self.root)
        self.assertFalse(ok)
        self.assertTrue(any("schema:" in l for l in lines), lines)

    def test_empty_criteria_fails(self):
        ok, lines = chk.evaluate(_doc([]), self.root)
        self.assertFalse(ok)
        self.assertTrue(any("criteria: missing or empty" in l for l in lines), lines)


class TestRealSelfCert(unittest.TestCase):
    """The committed self-cert must validate against the real repo tree."""

    def test_real_self_cert_passes(self):
        path = REPO_ROOT / "compliance" / "openssf" / "best-practices-self-cert.json"
        doc = chk.load(path)
        ok, lines = chk.evaluate(doc, REPO_ROOT)
        self.assertTrue(ok, "\n".join(lines))
        # The real file must cover all six badge families.
        fams = {c["family"] for c in doc["criteria"]}
        self.assertEqual(fams, chk.VALID_FAMILY, fams)
        # GX-4 is not yet filed — the file must stay honest about that.
        self.assertFalse(doc["project"]["filed"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
