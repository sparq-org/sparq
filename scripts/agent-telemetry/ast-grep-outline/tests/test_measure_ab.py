#!/usr/bin/env python3
# Hermetic tests for the read-payload measurer (bead sq-lhwo.4, epic
# sq-lhwo). 🤖 SPARQ agent.
#
# Hermetic where it matters: the byte->token proxy, the cache-discount formula, the
# hit-digest reducer, and the corpus drift-check are tested against a SYNTHETIC
# temp-dir corpus with NO dependency on ast-grep / grep being installed (those are
# exercised only by the optional live smoke test, which self-skips if ast-grep is
# absent). The committed gate is the hermetic part.
#
# Run:  python3 scripts/agent-telemetry/ast-grep-outline/tests/test_measure_ab.py

from __future__ import annotations

import hashlib
import importlib.util
import json
import shutil
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
MODULE = HERE.parent / "measure_ab.py"

spec = importlib.util.spec_from_file_location("measure_ab", MODULE)
m = importlib.util.module_from_spec(spec)
assert spec and spec.loader
spec.loader.exec_module(m)


class ProxyMathTest(unittest.TestCase):
    def test_chars_per_token_proxy(self):
        # 350 chars / 3.5 == 100 tokens.
        self.assertAlmostEqual(m._bytes_to_tokens(350), 100.0)

    def test_effective_formula_matches_metrics_row(self):
        # Mirror of the §5.1 formula: 1.0*fresh + 0.1*read + 1.25*write.
        self.assertAlmostEqual(
            m.effective_input_tokens(100, 200, 40), 100 + 20 + 50.0
        )

    def test_read_payload_is_all_fresh(self):
        # In the read-payload proxy a payload is pure fresh input; no cache modelled.
        self.assertAlmostEqual(m.effective_input_tokens(123, 0, 0), 123.0)


class DigestTest(unittest.TestCase):
    def test_digest_reduces_to_one_line_per_hit(self):
        raw = json.dumps([
            {"file": "crates/a.rs", "range": {"start": {"line": 41}},
             "lines": "pub fn foo() -> X {\n  body\n}"},
            {"file": "crates/b.rs", "range": {"start": {"line": 9}},
             "lines": "pub fn bar()"},
        ])
        out = m._digest_hits(raw)
        lines = out.splitlines()
        self.assertEqual(len(lines), 2)
        self.assertEqual(lines[0], "crates/a.rs:42: pub fn foo() -> X {")  # 0-based +1
        self.assertNotIn("body", out)  # body bytes are NOT in the answer-sized view

    def test_digest_empty_on_no_hits(self):
        self.assertEqual(m._digest_hits(""), "")
        self.assertEqual(m._digest_hits("[]"), "")


class DriftCheckTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self._orig_root = m.REPO_ROOT
        m.REPO_ROOT = self.tmp
        # synthetic corpus
        (self.tmp / "crates").mkdir()
        self.f = self.tmp / "crates" / "x.rs"
        self.f.write_text("pub fn a() {}\n", encoding="utf-8")

    def tearDown(self):
        m.REPO_ROOT = self._orig_root
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _manifest(self, sha: str) -> Path:
        mp = self.tmp / "manifest.json"
        mp.write_text(json.dumps({
            "_README": "synthetic",
            "crates/": {"dir": True, "note": "glob target"},
            "crates/x.rs": {"bytes": self.f.stat().st_size, "sha256": sha},
        }), encoding="utf-8")
        return mp

    def test_clean_manifest_no_drift(self):
        sha = hashlib.sha256(self.f.read_bytes()).hexdigest()
        drift = m.verify_corpus(self._manifest(sha))
        self.assertEqual(drift, [])  # README + dir entry skipped; file matches

    def test_content_drift_detected(self):
        drift = m.verify_corpus(self._manifest("0" * 64))
        self.assertEqual(len(drift), 1)
        self.assertTrue(drift[0].startswith("DRIFT:"))

    def test_missing_file_detected(self):
        mp = self._manifest("0" * 64)  # build the manifest while the file still exists
        self.f.unlink()                # then remove the file -> MISSING at check time
        drift = m.verify_corpus(mp)
        self.assertEqual(len(drift), 1)
        self.assertTrue(drift[0].startswith("MISSING:"))


class LiveSmokeTest(unittest.TestCase):
    """Optional: only runs if ast-grep is actually installed. Confirms the two arms
    execute over a tiny synthetic crate and the OUTLINE arm is strictly smaller than
    the FULL-READ arm (the genuine lever the skill claims)."""

    def setUp(self):
        try:
            self.ast_grep = m._ast_grep_bin()
        except RuntimeError:
            self.skipTest("ast-grep not installed — skipping live smoke")
        self.tmp = Path(tempfile.mkdtemp())
        self._orig_root = m.REPO_ROOT
        m.REPO_ROOT = self.tmp
        src = self.tmp / "crates" / "demo" / "src"
        src.mkdir(parents=True)
        body = "\n".join(f"    let v{i} = {i};" for i in range(200))
        (src / "big.rs").write_text(
            "pub fn entry() {\n" + body + "\n}\n"
            "pub fn helper() -> u32 { 0 }\n",
            encoding="utf-8",
        )

    def tearDown(self):
        if hasattr(self, "tmp"):
            m.REPO_ROOT = self._orig_root
            shutil.rmtree(self.tmp, ignore_errors=True)

    def test_outline_arm_smaller_than_full_read(self):
        task = {
            "id": "smoke", "stratum": "shape", "kind": "shape",
            "armA": {"read_file": "crates/demo/src/big.rs"},
            "armB": {"lang": "rust", "pattern": "pub fn $N($$$) $$$",
                     "outline_file": "crates/demo/src/big.rs"},
        }
        row = m.measure_task(task, self.ast_grep)
        self.assertGreater(row["arm_a_bytes"], row["arm_b_bytes"])
        self.assertGreater(row["delta_eff"], 0)  # B (outline) is cheaper


if __name__ == "__main__":
    unittest.main(verbosity=2)
