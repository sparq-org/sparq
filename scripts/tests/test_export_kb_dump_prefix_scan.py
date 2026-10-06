"""export-kb-dump.py restricted-projection scanner must catch auto-prefixed
restricted terms WITHOUT rdflib (#6037, #6230, #6383).

The stdlib prefix-expansion check is exercised directly, so this suite is
hermetic and passes identically whether or not rdflib is installed.
"""
import importlib.util
import pathlib
import subprocess
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "export-kb-dump.py"

_spec = importlib.util.spec_from_file_location("export_kb_dump", SCRIPT)
kb = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(kb)

KEY = "pkg-restricted-projection.ttl.gz"


def _stdlib_hits(content: str):
    return kb._prefix_expanded_check_restricted_projection(KEY, content)


class PrefixExpandedScan(unittest.TestCase):
    def test_probe_b_auto_prefix_caught(self):
        ttl = (
            "@prefix ns1: <https://w3id.org/zkp-sparql/sig-impl#> .\n"
            '<https://doi.org/10.5555/b> ns1:justification "x" .\n'
        )
        hits = _stdlib_hits(ttl)
        self.assertEqual(len(hits), 1)
        self.assertEqual(hits[0].marker, "ns1:justification")
        self.assertEqual(hits[0].line_no, 2)

    def test_sparql_style_prefix_and_other_terms_caught(self):
        ttl = (
            "PREFIX ns2: <http://purl.org/dc/terms/>\n"
            "prefix ns3: <https://sparq.dev/ns/pkg#>\n"
            '<https://doi.org/10.5555/c> a ns3:Finding ; ns2:abstract "y" .\n'
        )
        markers = sorted(h.marker for h in _stdlib_hits(ttl))
        self.assertEqual(markers, ["ns2:abstract", "ns3:Finding"])

    def test_empty_prefix_caught(self):
        ttl = (
            "@prefix : <https://w3id.org/zkp-sparql/sig-impl#> .\n"
            '<https://doi.org/10.5555/d> :justification "z" .\n'
        )
        self.assertEqual(len(_stdlib_hits(ttl)), 1)

    def test_clean_projection_not_flagged(self):
        ttl = (
            "@prefix ns1: <https://w3id.org/zkp-sparql/sig-impl#> .\n"
            "@prefix dcterms: <http://purl.org/dc/terms/> .\n"
            '<https://doi.org/10.5555/e> dcterms:title "t" ;\n'
            '  ns1:justificationCount 3 ; dcterms:abstractLength 9 .\n'
        )
        self.assertEqual(_stdlib_hits(ttl), [])

    def test_commented_rebinding_does_not_mask_the_real_binding(self):
        # Codex review: a commented-out directive must not overwrite the real one.
        ttl = (
            "@prefix ns1: <http://purl.org/dc/terms/> .\n"
            "# @prefix ns1: <https://example.org/> .\n"
            '<https://doi.org/10.5555/g> ns1:abstract "leak" .\n'
        )
        self.assertEqual([h.marker for h in _stdlib_hits(ttl)], ["ns1:abstract"])

    def test_directives_inside_literals_and_iris_are_ignored(self):
        ttl = (
            "@prefix ns1: <http://purl.org/dc/terms/> .\n"
            '<https://doi.org/10.5555/h> <https://ex.org/note> """multi\n'
            '@prefix ns1: <https://example.org/> .\n""" ;\n'
            "  <https://ex.org/n2> '@prefix ns1: <https://example.org/> .' ;\n"
            '  ns1:abstract "leak" .\n'
        )
        hits = _stdlib_hits(ttl)
        self.assertEqual([h.marker for h in hits], ["ns1:abstract"])
        self.assertEqual(hits[0].line_no, 6)

    def test_names_inside_literals_are_not_flagged(self):
        ttl = (
            "@prefix ns1: <http://purl.org/dc/terms/> .\n"
            '<https://doi.org/10.5555/i> <https://ex.org/note> "see ns1:abstract" .\n'
        )
        self.assertEqual(_stdlib_hits(ttl), [])

    def test_rebinding_uses_the_binding_in_force(self):
        ttl = (
            "@prefix ns1: <https://example.org/> .\n"
            '<https://doi.org/10.5555/j> ns1:abstract "harmless" .\n'
            "@prefix ns1: <http://purl.org/dc/terms/> .\n"
            '<https://doi.org/10.5555/j> ns1:abstract "leak" .\n'
            "PREFIX ns1: <https://example.org/>\n"
            '<https://doi.org/10.5555/j> ns1:abstract "harmless again" .\n'
        )
        self.assertEqual([h.line_no for h in _stdlib_hits(ttl)], [4])

    def test_longer_local_names_are_not_flagged(self):
        # `ns1:abstract:count` is a complete PN_LOCAL distinct from `ns1:abstract`;
        # a trailing `.` is the statement terminator, not part of the name.
        ttl = (
            "@prefix ns1: <http://purl.org/dc/terms/> .\n"
            '<https://doi.org/10.5555/k> ns1:abstract:count 3 ; ns1:abstract.v 1 .\n'
            "<https://doi.org/10.5555/k> <https://ex.org/p> ns1:abstract.\n"
        )
        hits = _stdlib_hits(ttl)
        self.assertEqual([(h.marker, h.line_no) for h in hits], [("ns1:abstract", 3)])

    def test_run_leak_check_catches_auto_prefix(self):
        ttl = (
            "@prefix ns1: <https://w3id.org/zkp-sparql/sig-impl#> .\n"
            '<https://doi.org/10.5555/f> ns1:justification "x" .\n'
        )
        passed, _ = kb.run_leak_check({KEY: ttl}, restricted_projection_key=KEY)
        self.assertFalse(passed)

    def test_dry_run_self_test_green(self):
        r = subprocess.run(
            [sys.executable, str(SCRIPT), "--dry-run"],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        self.assertEqual(r.returncode, 0, r.stdout[-2000:] + r.stderr[-2000:])
        self.assertIn("PROBE B (ns1: auto-prefix)] CAUGHT", r.stdout)


if __name__ == "__main__":
    unittest.main()
