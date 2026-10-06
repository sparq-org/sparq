#!/usr/bin/env python3
# Hermetic tests for the PKG write-path YAML-LD compiler
# (crates/sparq-kb/ingest/yamlld_compile.py; FO-bridge Phase 4, bead sq-mztg8.2, epic
# sq-mztg8). 🤖 SPARQ agent.
#
# Hermetic w.r.t. git/network/filesystem-state: imports the compiler module's pure
# entry points (parse_yaml_ld / ConceptResolver / compile_yaml_ld) and drives them over
# INLINE fixtures. NO subprocess, NO network. Stdlib-only (no PyYAML) — the compiler is
# stdlib-only by design, so the test is too.
#
# It pins the load-bearing write-path invariants the bead's acceptance criterion names:
#   - the guarded V() resolver: lexical resolution, AMBIGUOUS token -> hard error,
#     UNRESOLVABLE token -> hard error (the fail-closed contract: never a silent wrong
#     IRI), CURIE pass-through;
#   - the YAML-LD subset reader (mappings / block + flow sequences / typed scalars /
#     comments inside vs outside quotes);
#   - the deterministic Turtle emit (typed-key projection, enum mapping, xsd:decimal
#     formatting) and a round-trip on the real authoring source.
#
# Run:  python3 scripts/tests/test_yamlld_compile.py
# (stdlib only; no pytest required — also discoverable by `pytest`.)

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
COMPILER = REPO_ROOT / "crates" / "sparq-kb" / "ingest" / "yamlld_compile.py"
AUTHORING_SRC = REPO_ROOT / "crates" / "sparq-kb" / "ingest" / "agents-findings.yaml.ld"


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


yc = _load("yamlld_compile", COMPILER)


CATALOG = [
    {"@id": "kb:topic-merge-discipline", "prefLabel": "Merge discipline"},
    {"@id": "kb:topic-subagent-rules", "prefLabel": "Sub-agent standing rules"},
    {"@id": "kb:topic-zk-discipline", "prefLabel": "ZK/MPC claim + circuit discipline"},
]


class TestYamlSubsetReader(unittest.TestCase):
    def test_scalar_kinds(self):
        doc = yc.parse_yaml_ld(
            'a: hello\n'
            'b: "quoted: with colon"\n'
            'c: 0.97\n'
            'd: 3\n'
            'e: true\n'
        )
        self.assertEqual(doc["a"], "hello")
        self.assertEqual(doc["b"], "quoted: with colon")
        self.assertEqual(doc["c"], 0.97)
        self.assertEqual(doc["d"], 3)
        self.assertIs(doc["e"], True)

    def test_flow_sequence_and_block_sequence(self):
        doc = yc.parse_yaml_ld(
            'flow: [x, y, z]\n'
            'block:\n'
            '  - one\n'
            '  - two\n'
        )
        self.assertEqual(doc["flow"], ["x", "y", "z"])
        self.assertEqual(doc["block"], ["one", "two"])

    def test_flow_mapping(self):
        doc = yc.parse_yaml_ld('k: { "@id": "pkg:about", "@type": "@token" }\n')
        self.assertEqual(doc["k"], {"@id": "pkg:about", "@type": "@token"})

    def test_sequence_of_maps_preserves_order(self):
        doc = yc.parse_yaml_ld(
            'items:\n'
            '  - "@id": a\n'
            '    x: 1\n'
            '  - "@id": b\n'
            '    y: 2\n'
        )
        self.assertEqual(doc["items"], [{"@id": "a", "x": 1}, {"@id": "b", "y": 2}])

    def test_hash_inside_quotes_is_data_not_comment(self):
        doc = yc.parse_yaml_ld('a: "value # not a comment"  # real comment\n')
        self.assertEqual(doc["a"], "value # not a comment")

    def test_tab_indentation_is_rejected(self):
        with self.assertRaises(yc.CompileError):
            yc.parse_yaml_ld('a:\n\t- x\n')


class TestGuardedResolver(unittest.TestCase):
    def setUp(self):
        self.r = yc.ConceptResolver(CATALOG, echo=False)

    def test_local_name_resolution(self):
        self.assertEqual(
            self.r.resolve("topic-merge-discipline"), "kb:topic-merge-discipline"
        )

    def test_faceted_token_resolution(self):
        # the author writes the facet-stripped token; V() resolves it.
        self.assertEqual(
            self.r.resolve("merge-discipline"), "kb:topic-merge-discipline"
        )
        self.assertEqual(self.r.resolve("zk-discipline"), "kb:topic-zk-discipline")

    def test_preflabel_resolution_normalised(self):
        self.assertEqual(
            self.r.resolve("Merge Discipline"), "kb:topic-merge-discipline"
        )

    def test_unresolvable_token_is_hard_error(self):
        with self.assertRaises(yc.CompileError) as cm:
            self.r.resolve("no-such-concept")
        self.assertIn("unresolvable", str(cm.exception))

    def test_ambiguous_token_is_hard_error(self):
        # two distinct concept IRIs sharing a normalised key -> AMBIGUOUS -> hard error.
        ambig = [
            {"@id": "kb:topic-foo", "prefLabel": "Shared label"},
            {"@id": "kb:topic-bar", "prefLabel": "Shared label"},
        ]
        r = yc.ConceptResolver(ambig, echo=False)
        with self.assertRaises(yc.CompileError) as cm:
            r.resolve("shared-label")
        self.assertIn("AMBIGUOUS", str(cm.exception))

    def test_same_iri_two_labels_is_not_ambiguous(self):
        # one IRI reachable by two keys is NOT ambiguous (a set, not a list).
        one = [{"@id": "kb:topic-x", "prefLabel": "topic-x"}]
        r = yc.ConceptResolver(one, echo=False)
        self.assertEqual(r.resolve("topic-x"), "kb:topic-x")

    def test_known_curie_passes_through(self):
        # a CURIE not in the catalog (an ontology individual) passes through unchanged.
        self.assertEqual(self.r.resolve("pkg:Explored"), "pkg:Explored")


class TestCompileEnd2End(unittest.TestCase):
    def test_compile_emits_conformant_shaped_turtle(self):
        src = (
            '"@context":\n'
            '  "@vocab": "https://sparq.dev/ns/pkg#"\n'
            '  about: { "@id": "pkg:about", "@type": "@token" }\n'
            '  finding: { "@id": "rdfs:label", "@language": "en-GB" }\n'
            '  verdict: { "@id": "pkg:verdict", "@type": "@verdict" }\n'
            '  assurance: { "@id": "pkg:assurance", "@type": "@assurance" }\n'
            '  confidence: { "@id": "pkg:confidence", "@type": "xsd:decimal" }\n'
            '  justification: { "@id": "sigimpl:justification", "@language": "en-GB" }\n'
            '  source: { "@id": "prov:wasDerivedFrom", "@type": "@id" }\n'
            'concepts:\n'
            '  - "@id": kb:topic-x\n'
            '    "@type": skos:Concept\n'
            '    prefLabel: "Topic X"\n'
            '"@graph":\n'
            '  - "@id": kb:find-1\n'
            '    "@type": pkg:Finding\n'
            '    finding: "A finding about topic x"\n'
            '    about: topic-x\n'
            '    verdict: yes\n'
            '    assurance: claimed\n'
            '    confidence: 0.9\n'
            '    justification: "a sufficiently long non-filler justification string"\n'
            '    source: kb:doc-x\n'
        )
        ttl = yc.compile_yaml_ld(src, echo=False)
        # the verdict/assurance enums mapped, the token resolved, the decimal kept .9
        self.assertIn("pkg:about kb:topic-x", ttl)
        self.assertIn("pkg:verdict sigimpl:yes", ttl)
        self.assertIn("pkg:assurance secx:Claimed", ttl)
        self.assertIn("pkg:confidence 0.9", ttl)
        self.assertIn("prov:wasDerivedFrom kb:doc-x", ttl)
        self.assertIn('rdfs:label "A finding about topic x"@en-GB', ttl)

    def test_integer_confidence_forced_to_decimal(self):
        # SHACL FindingShape requires xsd:decimal; a bare `1` would be xsd:integer, so
        # the emitter forces `1.0`.
        self.assertEqual(yc._fmt_decimal(1), "1.0")
        self.assertEqual(yc._fmt_decimal(0.97), "0.97")

    def test_bad_verdict_is_hard_error(self):
        src = (
            '"@context":\n'
            '  verdict: { "@id": "pkg:verdict", "@type": "@verdict" }\n'
            '"@graph":\n'
            '  - "@id": kb:f\n'
            '    "@type": pkg:Finding\n'
            '    verdict: maybe\n'
        )
        with self.assertRaises(yc.CompileError):
            yc.compile_yaml_ld(src, echo=False)

    def test_node_without_type_is_hard_error(self):
        src = '"@graph":\n  - "@id": kb:f\n    label: x\n'
        with self.assertRaises(yc.CompileError):
            yc.compile_yaml_ld(src, echo=False)

    def test_real_authoring_source_compiles_with_11_findings(self):
        # the committed authoring source compiles cleanly and carries the 11 Findings.
        text = AUTHORING_SRC.read_text(encoding="utf-8")
        ttl = yc.compile_yaml_ld(text, echo=False)
        self.assertEqual(ttl.count("a pkg:Finding"), 11)
        # every Finding carries its four mandatory FindingShape fields (smoke check).
        self.assertEqual(ttl.count("pkg:confidence"), 12)  # 11 findings + 1 source
        self.assertEqual(ttl.count("pkg:assurance"), 11)
        self.assertEqual(ttl.count("sigimpl:justification"), 11)
        self.assertEqual(ttl.count("prov:wasDerivedFrom"), 11)


if __name__ == "__main__":
    unittest.main(verbosity=2)
