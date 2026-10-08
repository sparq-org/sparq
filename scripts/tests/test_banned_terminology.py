#!/usr/bin/env python3
# issue #3811 — the NAMED both-direction + wiring test for the banned-terminology
# HARD gate (scripts/check-terminology.py + scripts/banned-terminology.json).
#
# WHY THIS EXISTS: 98 occurrences of a maintainer-banned term reached PR #3451 as a `pub`
# Rust type AND a published `.ttl` rdfs:comment with a GREEN `ci-summary / gate`, because
# the terminology gate hard-coded its list and scanned only `*.md`. A gate that cannot
# demonstrate catching the thing it exists to catch is vacuous — that is the exact defect
# class that let this through — so every assertion here is a MUTATION anchor:
#
#   TestFixtureMutation   — delete/neuter the term declaration in banned-terminology.json
#                           => the violation fixtures stop being caught => RED.
#   TestSurfaceCoverage   — narrow the term's surface back to markdown-only (the #3811
#                           root cause) => `.rs`/`.ttl` leave the scanned set => RED.
#   TestEscapeDiscipline  — widen an escape (blanket allowPattern / broad exemptPath)
#                           => the violation fixtures pass => RED.
#   TestWorkflowWiring    — delete the gating STEP, its `run:` CALL SITE, add a skipping
#                           job-level `if:`, drop the merge_group trigger, or rename the
#                           job to look advisory => RED. (The YAML seam is where vacuity
#                           lives on this fleet; a check that does not execute in the
#                           gating matrix is not a gate.)
#
# Run:  python3 scripts/tests/test_banned_terminology.py

from __future__ import annotations

import importlib.util
import json
import re
import subprocess
import sys
import unittest
from pathlib import Path


# #5820: local runs without PyYAML skip this module instead of erroring; CI still
# hard-fails on a missing PyYAML (see scripts/tests/_yaml_seam.py).
sys.path.insert(0, str(Path(__file__).resolve().parent))
from _yaml_seam import yaml_or_local_skip  # noqa: E402

yaml = yaml_or_local_skip()

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
GATE_PY = REPO_ROOT / "scripts" / "check-terminology.py"
CONFIG = REPO_ROOT / "scripts" / "banned-terminology.json"
FIXTURES = REPO_ROOT / "scripts" / "tests" / "fixtures" / "banned-terminology"
DOCS_QUALITY_YML = REPO_ROOT / ".github" / "workflows" / "docs-quality.yml"

# The term this whole exercise is about (issue #3811).
TERM_ID = "trust-envelope"
# The two extensions whose ABSENCE from the gate's surface was the root cause.
ROOT_CAUSE_EXTS = (".rs", ".ttl")


def _load_gate():
    spec = importlib.util.spec_from_file_location("check_terminology", GATE_PY)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


GATE = _load_gate()


def run_gate(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(GATE_PY), *args],
        capture_output=True, text=True, cwd=REPO_ROOT,
    )


class TestFixtureMutation(unittest.TestCase):
    """The gate must actually catch the shapes that reached PR #3451."""

    def test_violation_fixtures_are_caught(self):
        for name in ("violation.rs", "violation.ttl", "violation.md"):
            with self.subTest(fixture=name):
                rel = f"scripts/tests/fixtures/banned-terminology/{name}"
                res = run_gate(rel)
                self.assertEqual(
                    res.returncode, 1,
                    f"{name} contains the banned term but the gate returned "
                    f"{res.returncode}.\nstdout:\n{res.stdout}",
                )
                self.assertIn("banned terminology", res.stdout)

    def test_public_rust_api_shape_is_caught(self):
        """The exact PR #3451 shape: the term as a `pub` type name in a .rs file."""
        res = run_gate("scripts/tests/fixtures/banned-terminology/violation.rs")
        self.assertEqual(res.returncode, 1)
        self.assertIn("pub struct", res.stdout)

    def test_published_vocabulary_shape_is_caught(self):
        """The other PR #3451 shape: the term inside a .ttl rdfs:comment."""
        res = run_gate("scripts/tests/fixtures/banned-terminology/violation.ttl")
        self.assertEqual(res.returncode, 1)
        self.assertIn("rdfs:comment", res.stdout)

    def test_every_spelling_is_caught(self):
        """Case-insensitive + the camel/snake/kebab/spaced spellings."""
        _, terms, _ = GATE.load_config()
        term = next(t for t in terms if t.id == TERM_ID)
        for spelling in (
            "TrustEnvelope", "trust_envelope", "trust-envelope",
            "trust envelope", "Trust Envelope", "TRUST ENVELOPE",
            "trustEnvelope", "parse_envelope is fine but trust_envelope is not",
        ):
            with self.subTest(spelling=spelling):
                self.assertTrue(
                    term.matches(spelling), f"{spelling!r} must be caught"
                )

    def test_approved_wording_and_unrelated_envelope_pass(self):
        res = run_gate(
            "scripts/tests/fixtures/banned-terminology/clean.rs",
            "scripts/tests/fixtures/banned-terminology/allowlisted.md",
        )
        self.assertEqual(
            res.returncode, 0,
            f"approved wording must not trip the gate.\nstdout:\n{res.stdout}",
        )

    def test_bare_envelope_is_not_banned(self):
        """`envelope` alone is legitimate repo-wide (leakage/JSON/SD-JWT/noise)."""
        _, terms, _ = GATE.load_config()
        term = next(t for t in terms if t.id == TERM_ID)
        for ok in (
            "pub struct Envelope { payload: Vec<u8> }",
            "the leakage envelope of the MPC routing seam",
            "Salted-hash selective-disclosure envelope over a JWT.",
            "scripts/bench/emit_envelope.py",
        ):
            with self.subTest(line=ok):
                self.assertFalse(term.matches(ok), f"{ok!r} must NOT be caught")

    def test_repo_wide_scan_is_clean(self):
        res = run_gate()
        self.assertEqual(
            res.returncode, 0,
            f"repo-wide terminology scan must be clean.\n{res.stdout}\n{res.stderr}",
        )


class TestSurfaceCoverage(unittest.TestCase):
    """Pins the #3811 ROOT CAUSE: the term must be checked in code + vocabulary."""

    def test_term_is_declared(self):
        cfg = json.loads(CONFIG.read_text())
        ids = [t["id"] for t in cfg["terms"]]
        self.assertIn(
            TERM_ID, ids,
            "the #3811 banned term must be declared in banned-terminology.json",
        )

    def test_term_surface_reaches_rust_and_turtle(self):
        """If someone narrows this surface back to markdown-only, this goes RED."""
        _, terms, _ = GATE.load_config()
        term = next(t for t in terms if t.id == TERM_ID)
        includes = set(term.surface["include"])
        for ext in ROOT_CAUSE_EXTS:
            self.assertIn(
                f"*{ext}", includes,
                f"the {TERM_ID} surface must include *{ext} — its absence IS the "
                "#3811 root cause (the term reached a green gate in that extension)",
            )

    def test_real_repo_rust_and_turtle_files_are_actually_scanned(self):
        """Surface globs must resolve to REAL tracked files, not just look right."""
        _, terms, _ = GATE.load_config()
        term = next(t for t in terms if t.id == TERM_ID)
        scanned = GATE.surface_files(term.surface)
        for ext in ROOT_CAUSE_EXTS:
            hits = [p for p in scanned if p.endswith(ext)]
            self.assertTrue(hits, f"no tracked {ext} file is in the scanned surface")
        # The very files PR #3451 touched must be inside the scanned surface.
        for must in (
            "crates/sparq-trust/src/expression.rs",
            "crates/sparq-trust/ontologies/trust/trust-framework.ttl",
        ):
            if (REPO_ROOT / must).exists():
                self.assertIn(
                    must, scanned, f"{must} must be inside the scanned surface"
                )

    def test_data_driven_not_hard_coded(self):
        """A future term must be a one-line JSON addition, not a code change."""
        src = GATE_PY.read_text()
        self.assertIn("banned-terminology.json", src)
        # The gate must not carry a hard-coded literal banned list.
        self.assertNotRegex(
            src.split("MUST_FAIL")[0],
            r"^BANNED\s*=\s*\[",
            "the banned list must live in JSON data, not in the script",
        )


class TestEscapeDiscipline(unittest.TestCase):
    """Escapes must stay narrow, enumerated and justified."""

    def test_every_exempt_path_carries_a_justification(self):
        cfg = json.loads(CONFIG.read_text())
        for term in cfg["terms"]:
            for entry in term.get("exemptPaths", []):
                with self.subTest(term=term["id"], path=entry.get("path")):
                    self.assertTrue(entry.get("why", "").strip(),
                                    "every exemptPaths entry needs a `why`")

    def test_exempt_paths_are_not_broad_directory_wildcards(self):
        """No `crates/**`-style blanket escape may be added to pass a violation."""
        cfg = json.loads(CONFIG.read_text())
        banned_shapes = {"**", "*", "crates/**", "src/**", "skills/**", "docs/**"}
        for term in cfg["terms"]:
            for entry in term.get("exemptPaths", []):
                path = entry["path"]
                with self.subTest(term=term["id"], path=path):
                    self.assertNotIn(path, banned_shapes)
                    # A glob is allowed only for the fixtures dir (which exists to
                    # hold violations); everything else must be an exact file.
                    if path.endswith("**"):
                        self.assertTrue(
                            path.startswith("scripts/tests/fixtures/"),
                            f"only the fixtures dir may use a glob exemption, got {path}",
                        )

    def test_allow_marker_requires_a_justification_shape(self):
        """The inline escape is `terminology-allow: <why>` — the colon is required."""
        allow_marker, terms, _ = GATE.load_config()
        term = next(t for t in terms if t.id == TERM_ID)
        self.assertTrue(term.line_allowed(f"TrustEnvelope {allow_marker}: recorded rename"))
        # A bare marker with no `:` justification does NOT exempt.
        self.assertFalse(term.line_allowed(f"TrustEnvelope {allow_marker}"))

    def test_term_has_no_blanket_allow_pattern(self):
        cfg = json.loads(CONFIG.read_text())
        self.assertEqual(
            cfg.get("allowPatterns", {}).get(TERM_ID, []), [],
            "the #3811 term must have no line-level allowPatterns — the inline "
            "marker is its only narrow escape",
        )

    def test_self_test_passes(self):
        res = run_gate("--self-test")
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)


class TestWorkflowWiring(unittest.TestCase):
    """The YAML seam. A gate that does not EXECUTE in the required matrix is not a gate.

    Every assertion here is a mutant killer for the seam the fleet keeps leaking at:
    the step, its `run:` call site, the job-level `if:`, and the trigger set.
    """

    @classmethod
    def setUpClass(cls):
        cls.raw = DOCS_QUALITY_YML.read_text()
        cls.doc = yaml.safe_load(cls.raw)
        cls.job = cls.doc["jobs"]["quick-gates"]
        cls.steps = cls.job["steps"]

    def _run_lines(self) -> str:
        return "\n".join(s.get("run", "") for s in self.steps if isinstance(s, dict))

    def test_gate_call_site_present(self):
        """MUTANT: delete the `run:` call site => RED."""
        # MULTILINE: the call site is one line inside the job's concatenated `run:` block.
        bare_call = re.compile(r"^\s*python3\s+scripts/check-terminology\.py\s*$", re.M)
        self.assertRegex(
            self._run_lines(), bare_call,
            "docs-quality quick-gates must INVOKE the terminology gate (bare, "
            "whole-surface run) — without this call site nothing is enforced",
        )

    def test_self_test_call_site_present(self):
        """MUTANT: delete the --self-test call site => RED."""
        self.assertIn("scripts/check-terminology.py --self-test", self._run_lines())

    def test_named_test_call_site_present(self):
        """MUTANT: delete THIS test's call site => RED (it would stop running in CI)."""
        self.assertIn(
            "scripts/tests/test_banned_terminology.py", self._run_lines(),
            "this named test must run in the gating job, or the fixtures prove nothing",
        )

    def test_gating_steps_have_no_skip_condition(self):
        """MUTANT: add `if:` to a gating step => RED."""
        for step in self.steps:
            run = step.get("run", "") if isinstance(step, dict) else ""
            if "check-terminology.py" in run or "test_banned_terminology.py" in run:
                with self.subTest(step=step.get("name")):
                    self.assertNotIn(
                        "if", step,
                        f"terminology step {step.get('name')!r} must not be conditional",
                    )

    def test_job_has_no_skip_condition(self):
        """MUTANT: add a job-level `if:` that can skip quick-gates => RED."""
        self.assertNotIn(
            "if", self.job,
            "docs-quality quick-gates must run unconditionally — a job-level `if:` "
            "could silently skip every HARD doc gate in the merge queue",
        )

    def test_job_is_not_advisory(self):
        """MUTANT: rename the job to `… (advisory)` => it stops gating => RED.

        ci_summary_gate.py treats a check whose DISPLAY NAME matches
        \\b(advisory|informational)\\b as non-gating.
        """
        name = self.job.get("name", "")
        self.assertNotRegex(
            name, r"\b(advisory|informational)\b",
            f"quick-gates job name {name!r} must not read as advisory",
        )

    def test_runs_in_the_required_matrix(self):
        """MUTANT: drop merge_group / pull_request => the gate stops guarding merges."""
        # `on:` parses as the boolean True key in YAML 1.1.
        triggers = self.doc.get("on", self.doc.get(True))
        self.assertIsNotNone(triggers, "workflow must declare triggers")
        for needed in ("pull_request", "merge_group"):
            self.assertIn(
                needed, triggers,
                f"docs-quality must trigger on {needed} — the merge queue evaluates "
                "`ci-summary / gate` there, and a gate absent from the queue is not a gate",
            )


if __name__ == "__main__":
    unittest.main(verbosity=2)
