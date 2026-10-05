#!/usr/bin/env python3
# sq-qcnn.31: Hermetic tests for check-feature-test-execution.py.
#
# Covers the acceptance criteria from the bead:
#   (a) --check GREEN on the real crates/ tree (all gated tests covered or allowlisted)
#   (b) --self-test RED on the negative fixture (detector correctly finds the violation)
#   (c) Unit tests for the core detection logic (FSM parser, feature closure, allowlist)
#
# All unit tests are hermetic: temporary directories with synthetic Rust/TOML files;
# no subprocess, no network, no PyYAML required for the unit test portion.
#
# Run:  python3 scripts/tests/test_check_feature_test_execution.py
# (stdlib + tomllib; also discoverable by pytest)

from __future__ import annotations

import importlib.util
import json
import os
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = REPO_ROOT / "scripts" / "check-feature-test-execution.py"


def _load_module():
    """Import the script module from its file path."""
    spec = importlib.util.spec_from_file_location("check_fte", SCRIPT)
    assert spec and spec.loader, "could not load {}".format(SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["check_fte"] = mod
    spec.loader.exec_module(mod)
    return mod


fte = _load_module()


# ---------------------------------------------------------------------------
# Helpers for building fixture crate directories
# ---------------------------------------------------------------------------

def _make_fixture_crate(
    base_dir: str,
    crate_name: str,
    features: dict | None = None,
    test_files: dict | None = None,
) -> Path:
    """Create a minimal fixture crate directory.

    features: {feature_name: list_of_implied_features, ...} — written into [features] table
    test_files: {relative_name: content_str, ...} — written under tests/
    """
    crate_dir = Path(base_dir) / crate_name

    # Write Cargo.toml
    toml_content = "[package]\nname = {!r}\nversion = \"0.1.0\"\nedition = \"2021\"\n".format(
        crate_name
    )
    if features:
        toml_content += "\n[features]\n"
        for feat, deps in features.items():
            toml_content += "{} = {}\n".format(feat, json.dumps(deps))

    cargo_toml = crate_dir / "Cargo.toml"
    cargo_toml.parent.mkdir(parents=True, exist_ok=True)
    cargo_toml.write_text(toml_content, encoding="utf-8")

    # Write test files
    if test_files:
        tests_dir = crate_dir / "tests"
        tests_dir.mkdir(parents=True, exist_ok=True)
        for fname, content in test_files.items():
            (tests_dir / fname).write_text(content, encoding="utf-8")

    return crate_dir


# ---------------------------------------------------------------------------
# Unit tests — feature closure computation
# ---------------------------------------------------------------------------

class TestComputeClosure(unittest.TestCase):
    def test_simple_implication(self):
        table = {"a": ["b"], "b": ["c"], "c": []}
        result = fte._compute_closure(frozenset(["a"]), table)
        self.assertIn("a", result)
        self.assertIn("b", result)
        self.assertIn("c", result)

    def test_dep_ref_not_followed(self):
        table = {"a": ["dep:some-crate", "b"]}
        result = fte._compute_closure(frozenset(["a"]), table)
        self.assertIn("b", result)
        self.assertNotIn("dep:some-crate", result)

    def test_cross_crate_ref_not_followed(self):
        table = {"a": ["other-crate/feature-x", "b"]}
        result = fte._compute_closure(frozenset(["a"]), table)
        self.assertIn("b", result)
        self.assertNotIn("other-crate/feature-x", result)

    def test_already_in_closure(self):
        table = {"a": ["b"]}
        result = fte._compute_closure(frozenset(["a", "b"]), table)
        self.assertEqual(result, frozenset({"a", "b"}))

    def test_empty_enabled(self):
        table = {"a": ["b"]}
        result = fte._compute_closure(frozenset(), table)
        self.assertEqual(result, frozenset())


# ---------------------------------------------------------------------------
# Unit tests — inner cfg feature extraction
# ---------------------------------------------------------------------------

class TestExtractInnerCfgFeatures(unittest.TestCase):
    def test_simple_feature(self):
        content = '#![cfg(feature = "my-feature")]\n\n#[test]\nfn foo() {}'
        result = fte._extract_inner_cfg_features(content)
        self.assertEqual(result, frozenset({"my-feature"}))

    def test_compound_all(self):
        content = '#![cfg(all(feature = "a", feature = "b"))]\n#[test]\nfn foo() {}'
        result = fte._extract_inner_cfg_features(content)
        self.assertEqual(result, frozenset({"a", "b"}))

    def test_compound_any_conservative(self):
        # any() treated conservatively: both features required
        content = '#![cfg(any(feature = "x", feature = "y"))]\n#[test]\nfn foo() {}'
        result = fte._extract_inner_cfg_features(content)
        self.assertEqual(result, frozenset({"x", "y"}))

    def test_no_feature_cfg(self):
        content = '#![cfg(test)]\n#[test]\nfn foo() {}'
        result = fte._extract_inner_cfg_features(content)
        self.assertEqual(result, frozenset())

    def test_outer_cfg_not_matched(self):
        # #[cfg(feature = "f")] (outer attribute) is NOT matched by the inner #![...] scanner
        content = '#[cfg(feature = "f")]\nfn foo() {}'
        result = fte._extract_inner_cfg_features(content)
        self.assertEqual(result, frozenset())

    def test_no_cfg_at_all(self):
        content = "fn foo() {}\n"
        result = fte._extract_inner_cfg_features(content)
        self.assertEqual(result, frozenset())


# ---------------------------------------------------------------------------
# Unit tests — scan_gated_tests
# ---------------------------------------------------------------------------

class TestScanGatedTests(unittest.TestCase):
    def test_simple_gated_test_file(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            _make_fixture_crate(
                tmpdir,
                "my-crate",
                features={"my-feat": []},
                test_files={"gated_test.rs": '#![cfg(feature = "my-feat")]\n#[test]\nfn t() {}'},
            )
            results = fte.scan_gated_tests(Path(tmpdir))
            self.assertEqual(len(results), 1)
            crate, test_id, feats = results[0]
            self.assertEqual(crate, "my-crate")
            self.assertEqual(test_id, "tests/gated_test.rs")
            self.assertEqual(feats, frozenset({"my-feat"}))

    def test_ungated_test_file_not_returned(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            _make_fixture_crate(
                tmpdir,
                "my-crate",
                test_files={"plain.rs": "#[test]\nfn t() {}"},
            )
            results = fte.scan_gated_tests(Path(tmpdir))
            self.assertEqual(results, [])

    def test_cargo_toml_required_features(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            crate_dir = Path(tmpdir) / "my-crate"
            crate_dir.mkdir()
            cargo_toml = crate_dir / "Cargo.toml"
            cargo_toml.write_text(
                textwrap.dedent("""\
                [package]
                name = "my-crate"
                version = "0.1.0"
                edition = "2021"

                [features]
                my-feat = []

                [[test]]
                name = "my_test"
                required-features = ["my-feat"]
                """),
                encoding="utf-8",
            )
            results = fte.scan_gated_tests(Path(tmpdir))
            self.assertEqual(len(results), 1)
            crate, test_id, feats = results[0]
            self.assertEqual(crate, "my-crate")
            self.assertEqual(test_id, "[[test]] my_test")
            self.assertEqual(feats, frozenset({"my-feat"}))

    def test_deduplication(self):
        """Two #![cfg(feature = ...)] in one file produce one entry."""
        with tempfile.TemporaryDirectory() as tmpdir:
            _make_fixture_crate(
                tmpdir,
                "my-crate",
                features={"a": [], "b": []},
                test_files={
                    "dup.rs": (
                        '#![cfg(feature = "a")]\n'
                        '#![cfg(feature = "b")]\n'
                        "#[test]\nfn t() {}"
                    )
                },
            )
            results = fte.scan_gated_tests(Path(tmpdir))
            # One entry, union of both features
            self.assertEqual(len(results), 1)
            _, _, feats = results[0]
            self.assertIn("a", feats)
            self.assertIn("b", feats)


# ---------------------------------------------------------------------------
# Unit tests — allowlist matching
# ---------------------------------------------------------------------------

class TestAllowlistCovers(unittest.TestCase):
    def test_exact_match_covers(self):
        entry = {"crate": "my-crate", "features": ["a", "b"], "reason": "test"}
        self.assertTrue(
            fte._allowlist_covers(entry, "my-crate", frozenset({"a", "b"}))
        )

    def test_wrong_crate_no_cover(self):
        entry = {"crate": "other", "features": ["a"], "reason": "test"}
        self.assertFalse(fte._allowlist_covers(entry, "my-crate", frozenset({"a"})))

    def test_subset_not_covered(self):
        # Entry {a,b} does NOT cover test requiring only {a} (exact match required)
        entry = {"crate": "my-crate", "features": ["a", "b"], "reason": "test"}
        self.assertFalse(fte._allowlist_covers(entry, "my-crate", frozenset({"a"})))

    def test_superset_not_covered(self):
        # Entry {a} does NOT cover test requiring {a, b}
        entry = {"crate": "my-crate", "features": ["a"], "reason": "test"}
        self.assertFalse(fte._allowlist_covers(entry, "my-crate", frozenset({"a", "b"})))


# ---------------------------------------------------------------------------
# Integration tests — check() function with synthetic crate dirs
# ---------------------------------------------------------------------------

class TestCheckFunction(unittest.TestCase):
    def test_all_covered_by_executor(self):
        """A gated test whose features are enabled by an executor passes."""
        with tempfile.TemporaryDirectory() as tmpdir:
            crates_dir = Path(tmpdir) / "crates"
            _make_fixture_crate(
                str(crates_dir),
                "my-crate",
                features={"my-feat": []},
                test_files={"gated.rs": '#![cfg(feature = "my-feat")]\n#[test]\nfn t() {}'},
            )
            # Executor: activates my-feat (no default features for this minimal crate)
            all_executors = {"my-crate": [frozenset({"my-feat"})]}
            allowlist: list = []
            gated = fte.scan_gated_tests(crates_dir)

            findings = []
            for crate, test_id, required in gated:
                if not fte._is_covered(crate, required, all_executors):
                    if not any(fte._allowlist_covers(e, crate, required) for e in allowlist):
                        findings.append("UNMAPPED: {}".format(test_id))

            self.assertEqual(findings, [])

    def test_uncovered_gated_test_found(self):
        """A gated test with no executor is reported as UNMAPPED."""
        with tempfile.TemporaryDirectory() as tmpdir:
            crates_dir = Path(tmpdir) / "crates"
            _make_fixture_crate(
                str(crates_dir),
                "my-crate",
                features={"ungated-feat": []},
                test_files={"ungated.rs": '#![cfg(feature = "ungated-feat")]\n#[test]\nfn t() {}'},
            )
            all_executors: dict = {}  # no executors
            allowlist: list = []
            gated = fte.scan_gated_tests(crates_dir)

            findings = []
            for crate, test_id, required in gated:
                if not fte._is_covered(crate, required, all_executors):
                    if not any(fte._allowlist_covers(e, crate, required) for e in allowlist):
                        findings.append(
                            "UNMAPPED: {}/{} requires features {{{}}}".format(
                                crate, test_id, ", ".join(sorted(required))
                            )
                        )

            self.assertEqual(len(findings), 1)
            self.assertIn("ungated-feat", findings[0])

    def test_allowlist_covers_uncovered(self):
        """An uncovered gated test with an allowlist entry does NOT appear in findings."""
        with tempfile.TemporaryDirectory() as tmpdir:
            crates_dir = Path(tmpdir) / "crates"
            _make_fixture_crate(
                str(crates_dir),
                "my-crate",
                features={"exotic": []},
                test_files={"exotic.rs": '#![cfg(feature = "exotic")]\n#[test]\nfn t() {}'},
            )
            all_executors: dict = {}
            allowlist = [{"crate": "my-crate", "features": ["exotic"], "reason": "wasm-only"}]
            gated = fte.scan_gated_tests(crates_dir)

            findings = []
            for crate, test_id, required in gated:
                if not fte._is_covered(crate, required, all_executors):
                    if not any(fte._allowlist_covers(e, crate, required) for e in allowlist):
                        findings.append("UNMAPPED: {}".format(test_id))

            self.assertEqual(findings, [])

    def test_transitive_feature_covered(self):
        """A test requiring feature B is covered by an executor that enables A (where A=>B)."""
        with tempfile.TemporaryDirectory() as tmpdir:
            crates_dir = Path(tmpdir) / "crates"
            # Feature A implies B; test only requires B; executor provides A
            crate_dir = Path(str(crates_dir)) / "my-crate"
            crate_dir.mkdir(parents=True)
            (crate_dir / "Cargo.toml").write_text(
                textwrap.dedent("""\
                [package]
                name = "my-crate"
                version = "0.1.0"
                edition = "2021"

                [features]
                a = ["b"]
                b = []
                """),
                encoding="utf-8",
            )
            tests_dir = crate_dir / "tests"
            tests_dir.mkdir()
            (tests_dir / "needs_b.rs").write_text(
                '#![cfg(feature = "b")]\n#[test]\nfn t() {}',
                encoding="utf-8",
            )
            # Executor activates "a" (which implies "b" via closure)
            activated = fte.executor_feature_set("my-crate", "a", crates_dir)
            self.assertIn("b", activated)

            all_executors = {"my-crate": [activated]}
            allowlist: list = []
            gated = fte.scan_gated_tests(crates_dir)

            findings = []
            for crate, test_id, required in gated:
                if not fte._is_covered(crate, required, all_executors):
                    if not any(fte._allowlist_covers(e, crate, required) for e in allowlist):
                        findings.append("UNMAPPED: {}".format(test_id))

            self.assertEqual(findings, [])


# ---------------------------------------------------------------------------
# Integration tests — coverage.sh FSM parser (synthetic coverage.sh snippets)
# ---------------------------------------------------------------------------

class TestCoverageFsmParser(unittest.TestCase):
    def _write_fake_coverage_sh(self, tmpdir: str, content: str) -> Path:
        p = Path(tmpdir) / "coverage.sh"
        p.write_text(content, encoding="utf-8")
        return p

    def test_single_arm_with_features(self):
        content = textwrap.dedent("""\
        PER_COMMIT_CRATES=(
          sparq-foo
        )

        measure() {
          local crate="$1"
          case "$crate" in
            sparq-foo)
              cargo_args+=(--features bar,baz); features+=("bar" "baz") ;;
          esac
        }
        """)
        with tempfile.TemporaryDirectory() as tmpdir:
            cov_sh = self._write_fake_coverage_sh(tmpdir, content)
            extras = fte._parse_coverage_case_extras(cov_sh)
            self.assertEqual(extras.get("sparq-foo"), frozenset({"bar", "baz"}))

    def test_arm_with_no_features(self):
        content = textwrap.dedent("""\
        PER_COMMIT_CRATES=(
          sparq-bar
        )

        measure() {
          local crate="$1"
          case "$crate" in
            sparq-bar)
              if [ "$TIER" = "per-commit" ]; then
                subcmd="test"
              fi ;;
          esac
        }
        """)
        with tempfile.TemporaryDirectory() as tmpdir:
            cov_sh = self._write_fake_coverage_sh(tmpdir, content)
            extras = fte._parse_coverage_case_extras(cov_sh)
            self.assertEqual(extras.get("sparq-bar"), frozenset())

    def test_comment_crate_ref_not_matched(self):
        # Comment text "sparq-engine)" inside an arm must NOT be treated as a new arm header
        content = textwrap.dedent("""\
        PER_COMMIT_CRATES=(
          sparq-engine-serialize
        )

        measure() {
          local crate="$1"
          case "$crate" in
            sparq-engine-serialize)
              # gating it had inside sparq-engine), so a default-feature build is empty
              cargo_args+=(--features serialize-rdf); features+=("serialize-rdf") ;;
          esac
        }
        """)
        with tempfile.TemporaryDirectory() as tmpdir:
            cov_sh = self._write_fake_coverage_sh(tmpdir, content)
            extras = fte._parse_coverage_case_extras(cov_sh)
            # Only sparq-engine-serialize arm found; sparq-engine from the comment NOT added
            self.assertIn("sparq-engine-serialize", extras)
            self.assertNotIn("sparq-engine", extras)
            self.assertEqual(extras["sparq-engine-serialize"], frozenset({"serialize-rdf"}))

    def test_multiline_features(self):
        content = textwrap.dedent("""\
        PER_COMMIT_CRATES=(
          sparq-multi
        )

        measure() {
          local crate="$1"
          case "$crate" in
            sparq-multi)
              cargo_args+=(--features feat1,feat2)
              features+=("feat1" "feat2") ;;
          esac
        }
        """)
        with tempfile.TemporaryDirectory() as tmpdir:
            cov_sh = self._write_fake_coverage_sh(tmpdir, content)
            extras = fte._parse_coverage_case_extras(cov_sh)
            self.assertEqual(extras.get("sparq-multi"), frozenset({"feat1", "feat2"}))

    def test_per_commit_crates_no_comments(self):
        content = textwrap.dedent("""\
        PER_COMMIT_CRATES=(
          # this is a comment sparq-commented
          sparq-real1 sparq-real2
          sparq-real3  # another comment sparq-ignored
        )
        """)
        with tempfile.TemporaryDirectory() as tmpdir:
            cov_sh = self._write_fake_coverage_sh(tmpdir, content)
            crates = fte._parse_per_commit_crates(cov_sh)
            self.assertIn("sparq-real1", crates)
            self.assertIn("sparq-real2", crates)
            self.assertIn("sparq-real3", crates)
            self.assertNotIn("sparq-commented", crates)
            self.assertNotIn("sparq-ignored", crates)


# ---------------------------------------------------------------------------
# Acceptance tests — --check and --self-test on real repo
# ---------------------------------------------------------------------------

class TestAcceptance(unittest.TestCase):
    """Integration tests against the real repo tree. Slow but load-bearing."""

    def test_check_green_on_real_tree(self):
        """--check must be GREEN (exit 0) on the current crates/ tree."""
        ok, findings = fte.check(
            fte.CRATES_DIR,
            fte.FRAGMENT_DIR,
            fte.COVERAGE_SH,
            fte.ALLOWLIST_PATH,
        )
        if not ok:
            self.fail(
                "--check is RED on the real tree; {} violation(s) found:\n{}".format(
                    len(findings), "\n".join("  " + f for f in findings)
                )
            )

    def test_self_test_passes(self):
        """--self-test must exit 0 (detector correctly finds the negative fixture violation)."""
        ok = fte.run_self_test(fte.FIXTURE_CRATES_DIR)
        self.assertTrue(
            ok,
            "--self-test FAILED: detector did not find the negative fixture violation; "
            "the guard is broken",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
