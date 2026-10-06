"""export-kb-dump.py must FAIL CLOSED without rdflib (#6037, #6230, #6383).

The string patterns cannot see prefixed forms such as "ns1:justification", so
without rdflib run_leak_check refuses the export, and the --dry-run self-test
reports the rdflib-only probe as SKIPPED rather than EVADED.
"""
import contextlib
import importlib.util
import io
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "export-kb-dump.py"

_spec = importlib.util.spec_from_file_location("export_kb_dump", SCRIPT)
kb = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(kb)

KEY = "pkg-restricted-projection.ttl.gz"
CLEAN = (
    "@prefix dcterms: <http://purl.org/dc/terms/> .\n"
    '<https://doi.org/10.5555/a> dcterms:title "t" .\n'
)
AUTO_PREFIX_LEAK = (
    "@prefix ns1: <https://w3id.org/zkp-sparql/sig-impl#> .\n"
    '<https://doi.org/10.5555/b> ns1:justification "x" .\n'
)

try:
    import rdflib  # noqa: F401

    HAVE_RDFLIB = True
except ImportError:
    HAVE_RDFLIB = False


def _check(content: str):
    err = io.StringIO()
    with contextlib.redirect_stderr(err):
        passed, viols = kb.run_leak_check({KEY: content}, restricted_projection_key=KEY)
    return passed, viols, err.getvalue()


def _no_rdflib():
    # `None` in sys.modules makes `import rdflib` raise ImportError.
    return mock.patch.dict(sys.modules, {"rdflib": None})


class WithoutRdflib(unittest.TestCase):
    def test_clean_projection_is_refused(self):
        with _no_rdflib():
            passed, viols, err = _check(CLEAN)
        self.assertFalse(passed)
        self.assertEqual([v.marker for v in viols], [kb.RDFLIB_REQUIRED_MARKER])
        self.assertIn("::error::rdflib is required", err)
        self.assertIn("pip install rdflib", err)

    def test_auto_prefix_leak_is_refused(self):
        with _no_rdflib():
            passed, _, _ = _check(AUTO_PREFIX_LEAK)
        self.assertFalse(passed)

    def test_string_patterns_still_run(self):
        leak = '<https://doi.org/10.5555/c> <http://purl.org/dc/terms/abstract> "x" .\n'
        with _no_rdflib():
            _, viols, _ = _check(leak)
        self.assertIn("dc/terms/abstract", [v.marker for v in viols])

    def test_files_other_than_the_projection_need_no_rdflib(self):
        err = io.StringIO()
        with _no_rdflib(), contextlib.redirect_stderr(err):
            passed, _ = kb.run_leak_check({"manifest.json": "{}"}, restricted_projection_key=KEY)
        self.assertTrue(passed)

    def test_dry_run_reports_probe_b_skipped_and_stays_green(self):
        with tempfile.TemporaryDirectory() as d:
            (pathlib.Path(d) / "rdflib.py").write_text("raise ImportError('simulated')\n")
            env = dict(os.environ, PYTHONPATH=d)
            r = subprocess.run([sys.executable, str(SCRIPT), "--dry-run"], cwd=ROOT,
                               env=env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout[-2000:] + r.stderr[-2000:])
        self.assertIn("Step 0 PASSED — export refused without rdflib", r.stdout)
        self.assertIn("[PROBE B (ns1: auto-prefix)] SKIPPED", r.stdout)
        self.assertNotIn("EVADED", r.stdout + r.stderr)


@unittest.skipUnless(HAVE_RDFLIB, "rdflib not installed")
class WithRdflib(unittest.TestCase):
    def test_clean_projection_passes(self):
        passed, viols, _ = _check(CLEAN)
        self.assertTrue(passed, [str(v) for v in viols])

    def test_auto_prefix_leak_is_caught_by_content(self):
        passed, viols, _ = _check(AUTO_PREFIX_LEAK)
        self.assertFalse(passed)
        self.assertNotIn(kb.RDFLIB_REQUIRED_MARKER, [v.marker for v in viols])

    def test_dry_run_catches_probe_b(self):
        r = subprocess.run([sys.executable, str(SCRIPT), "--dry-run"], cwd=ROOT,
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout[-2000:] + r.stderr[-2000:])
        self.assertIn("[PROBE B (ns1: auto-prefix)] CAUGHT", r.stdout)


if __name__ == "__main__":
    unittest.main()
