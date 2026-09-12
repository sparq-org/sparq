#!/usr/bin/env python3
# [OPUS-4.8] sq-toze.26 (GS-1 / NTIA N1): hermetic tests for scripts/check-sbom-supplier.py.
# Authored by Opus 4.8 (Fable unavailable; flag for re-review when Fable returns).
#
# Hermetic w.r.t. git/network: imports the check module and drives its pure evaluate() +
# the file parser against in-tmpdir fixtures. NO subprocess, NO live git. A final test
# generates the REAL workspace SBOM through scripts/sbom-normalize.jq (the exact
# publication transform) and asserts EVERY cargo component carries a supplier AND that the
# derivation is honest (registry/vendored -> "crates.io"; first-party sparq-* -> the
# project; never fabricated). That last test is skipped (not failed) if cargo-cyclonedx / jq
# are unavailable, so the suite stays runnable in a bare environment while still proving the
# real invariant where the toolchain exists (as on the CI runner).
#
# Run:  python3 scripts/tests/test_sbom_supplier.py
# (stdlib only; no pytest required — also discoverable by `pytest`.)

from __future__ import annotations

import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent

# [OPUS-4.8] sq-90ew: `cargo cyclonedx` internally runs `cargo metadata`, whose crates.io
# fetch is TRANSIENTLY flaky on hosted runners (observed `curl ... [16] Error in the HTTP2
# framing layer` -> non-zero exit at ~9-12s). PR #750 wrapped the cyclonedx invocation in the
# WORKFLOW STEPS, but this live-SBOM self-test runs its OWN `cargo cyclonedx` (it executes
# BEFORE those wrapped steps), so the flake still failed the GATING GS-1 job on unrelated PRs
# (main runs #27810325920 / #27811535047, post-#750). Mirror the #750 shell retry here:
# bounded 3 attempts, fixed 10s sleep, retry ONLY the GENERATION subprocess. This wraps NO
# assertion — a real GS-1 supplier-name violation is judged AFTER generation, on the produced
# files, and still fails. A genuine outage (all 3 attempts fail) re-raises -> the test errors.
_CYCLONEDX_ATTEMPTS = 3
_CYCLONEDX_RETRY_SLEEP_S = 10


def _generate_workspace_sbom() -> None:
    """Run `cargo cyclonedx --all` (writes <crate>.cdx.json next to each Cargo.toml), with a
    bounded retry around ONLY the transient crates.io-fetch flake. Re-raises the last
    CalledProcessError if every attempt fails (a genuine outage / toolchain break)."""
    last_exc: subprocess.CalledProcessError | None = None
    for attempt in range(1, _CYCLONEDX_ATTEMPTS + 1):
        try:
            subprocess.run(
                ["cargo", "cyclonedx", "--all", "--format", "json",
                 "--spec-version", "1.5"],
                cwd=REPO_ROOT, check=True, capture_output=True,
            )
            return
        except subprocess.CalledProcessError as exc:
            last_exc = exc
            if attempt >= _CYCLONEDX_ATTEMPTS:
                break
            print(f"cargo cyclonedx attempt {attempt} failed; retrying in "
                  f"{_CYCLONEDX_RETRY_SLEEP_S}s...", file=sys.stderr)
            time.sleep(_CYCLONEDX_RETRY_SLEEP_S)
    assert last_exc is not None
    raise last_exc


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, REPO_ROOT / "scripts" / filename)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


chk = _load("check_sbom_supplier", "check-sbom-supplier.py")


def _comp(name: str, *, supplier: dict | None = None, purl: str | None = None) -> dict:
    c: dict = {"type": "library", "name": name,
               "purl": purl if purl is not None else f"pkg:cargo/{name}@1.0.0"}
    if supplier is not None:
        c["supplier"] = supplier
    return c


class TestEvaluate(unittest.TestCase):
    def test_all_have_supplier_passes(self):
        ok, lines = chk.evaluate([
            _comp("serde", supplier={"name": "crates.io"}),
            _comp("sparq-core", supplier={"name": "Jesse Wright"}),
        ])
        self.assertTrue(ok, lines)
        self.assertIn("supplier.name", lines[0])

    def test_missing_supplier_fails(self):
        ok, lines = chk.evaluate([
            _comp("serde", supplier={"name": "crates.io"}),
            _comp("axum"),  # no supplier
        ])
        self.assertFalse(ok)
        self.assertTrue(any("MISSING per-component supplier" in l for l in lines))
        self.assertTrue(any("axum" in l for l in lines))

    def test_empty_supplier_name_fails(self):
        # A supplier object with a blank name is NOT a real Supplier Name.
        ok, _ = chk.evaluate([_comp("serde", supplier={"name": "   "})])
        self.assertFalse(ok)

    def test_supplier_without_name_fails(self):
        ok, _ = chk.evaluate([_comp("serde", supplier={"url": ["https://x"]})])
        self.assertFalse(ok)

    def test_non_cargo_component_is_ignored(self):
        # A component with no cargo purl is not judged (no cargo components -> fails-empty).
        ok, lines = chk.evaluate([{"type": "library", "name": "x"}])
        self.assertFalse(ok)
        self.assertTrue(any("No cargo components" in l for l in lines))

    def test_empty_fails(self):
        ok, _ = chk.evaluate([])
        self.assertFalse(ok)


class TestFileLevel(unittest.TestCase):
    def test_metadata_component_and_subcomponents_are_judged(self):
        # A missing supplier hiding ONLY on the root metadata.component (or its nested
        # build-target sub-components) must be caught.
        doc = {
            "bomFormat": "CycloneDX", "specVersion": "1.5",
            "metadata": {"component": {
                "type": "application", "name": "sparq-server",
                "purl": "pkg:cargo/sparq-server@0.1.0",  # NO supplier
                "components": [{"type": "library", "name": "sparq_server",
                                "purl": "pkg:cargo/sparq-server@0.1.0",
                                "supplier": {"name": "Jesse Wright"}}],
            }},
            "components": [{"type": "library", "name": "serde",
                            "purl": "pkg:cargo/serde@1.0.0",
                            "supplier": {"name": "crates.io"}}],
        }
        comps = chk._walk_components(doc)
        ok, lines = chk.evaluate(comps)
        self.assertFalse(ok)
        self.assertTrue(any("sparq-server" in l for l in lines))

    def test_vex_like_doc_has_no_cargo_components(self):
        doc = {"bomFormat": "CycloneDX", "specVersion": "1.5",
               "vulnerabilities": [{"id": "RUSTSEC-2024-0436"}]}
        self.assertEqual(chk._walk_components(doc), [])


class TestLiveWorkspaceSBOM(unittest.TestCase):
    """Generate the REAL normalized workspace SBOM and assert every cargo component carries
    an HONESTLY-derived supplier — the exact invariant the CI gate checks. Skipped if the
    toolchain is absent."""

    def test_real_normalized_sbom_has_honest_suppliers(self):
        if not shutil.which("cargo-cyclonedx") or not shutil.which("jq"):
            self.skipTest("cargo-cyclonedx / jq not available")
        normalize = REPO_ROOT / "scripts" / "sbom-normalize.jq"
        with tempfile.TemporaryDirectory() as d:
            # [OPUS-4.8] sq-90ew: GENERATION only, with the transient-flake retry. The GS-1
            # supplier assertion below runs ONCE on the produced files (NOT retried).
            _generate_workspace_sbom()
            try:
                judged = 0
                for crate in ("sparq-server", "sparq-cli"):
                    src = REPO_ROOT / "crates" / crate / f"{crate}.cdx.json"
                    if not src.exists():
                        continue
                    out = subprocess.run(
                        ["jq", "-f", str(normalize), str(src)],
                        check=True, capture_output=True, text=True,
                    ).stdout
                    doc = json.loads(out)
                    comps = chk._walk_components(doc)
                    ok, lines = chk.evaluate(comps)
                    self.assertTrue(ok, f"{crate}: {lines}")
                    # Honesty: only the two derivable supplier names appear, and the
                    # vendored upstream crate is NOT attributed to the project.
                    cargo = [c for c in comps
                             if str(c.get("purl", "")).startswith("pkg:cargo/")]
                    names = {chk._supplier_name(c) for c in cargo}
                    self.assertTrue(names <= {"crates.io", "Jesse Wright"},
                                    f"{crate}: unexpected supplier names {names}")
                    by_name = {c.get("name"): c for c in cargo}
                    if "spargebra" in by_name:
                        # vendored [patch.crates-io] crate -> crates.io, NEVER the project.
                        self.assertEqual(
                            chk._supplier_name(by_name["spargebra"]), "crates.io",
                            "vendored spargebra must be supplied by crates.io, not sparq",
                        )
                    if "sparq-core" in by_name:
                        self.assertEqual(
                            chk._supplier_name(by_name["sparq-core"]), "Jesse Wright",
                        )
                    judged += 1
                self.assertGreater(judged, 0, "no released-binary SBOM was generated")
            finally:
                for f in REPO_ROOT.glob("crates/**/*.cdx.json"):
                    f.unlink()


class TestDetachedSuppliers(unittest.TestCase):
    """[GPT-6] Explicit first-party members and patched-byte distributor identity."""

    def test_detached_and_patched_sources_keep_honest_supplier_identity(self):
        if not shutil.which("jq"):
            self.skipTest("jq not available")
        pairs = [
            ("sparq-proved-evaluator", "zk/sparql-evaluator/host", "Jesse Wright"),
            ("sparq-proved-evaluator-model", "zk/sparql-evaluator/model", "Jesse Wright"),
            ("sparq-proved-evaluator-methods", "zk/sparql-evaluator/methods", "Jesse Wright"),
            ("sparq-exact-guest", "zk/sparql-evaluator/methods/guest", "Jesse Wright"),
            ("sparq_proved_evaluator", "zk/sparql-evaluator/host", "Jesse Wright"),
            ("sparq_proved_evaluator_model", "zk/sparql-evaluator/model", "Jesse Wright"),
            ("sparq_proved_evaluator_methods", "zk/sparql-evaluator/methods", "Jesse Wright"),
            ("ark-relations", "vendor/zk-sdk/ark-relations-0.5.1", "Jesse Wright"),
            ("spargebra", "vendor/spargebra", "crates.io"),
            ("unrelated", "zk/sparql-evaluator/model", None),
            ("sparq-proved-evaluator-model", "elsewhere/model", None),
        ]
        raw = {"components": [{"name": name, "bom-ref": f"path+file:///build/{path}#0.1.0"}
                               for name, path, _ in pairs]}
        output = subprocess.check_output(
            ["jq", "-f", str(REPO_ROOT / "scripts/sbom-normalize.jq")],
            input=json.dumps(raw).encode(),
        )
        normalized = json.loads(output)
        for component, (_, _, supplier) in zip(normalized["components"], pairs):
            self.assertEqual(component.get("supplier", {}).get("name"), supplier)
        twice = subprocess.check_output(
            ["jq", "-f", str(REPO_ROOT / "scripts/sbom-normalize.jq")], input=output,
        )
        self.assertEqual(output, twice)


if __name__ == "__main__":
    unittest.main(verbosity=2)
