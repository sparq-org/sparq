#!/usr/bin/env python3
# [SONNET-4.6] Hermetic tests for the feature-matrix tier detector (bead sq-6g9kr).
# Design: research/feature-matrix-pyramid.md §4.
# Authored under the proceed-and-document rule.
#
# Covers the acceptance criteria:
#   (a) seeded cfg(feature)-gated #[test] in tests/ => SENSITIVE
#   (b) seeded cfg(not(feature)) in src/ => SENSITIVE
#   (c) nested any()/all() combinators parsed correctly => SENSITIVE
#   (d) parse error in cfg expression => SENSITIVE (fail-closed)
#   (e) unreadable crate dir => SENSITIVE (fail-closed)
#   (f) tier:check + sensitive verdict => enforce exit != 0
#   (g) mutation check: fail-open copy => non-sensitive on error
#   (h) non-sensitive fixture (no cfg refs) => NOT sensitive
#   (i) cfg(all(test, feature)) in src/ => SENSITIVE (b-direct pattern)
#   (j) a test:true leg for the same feature NAME in a DIFFERENT crate does not
#       satisfy the sq-vya1 guard (invariant (2) keys on (crate, feature))
#
# All tests are hermetic: they create temporary directories with synthetic
# Rust files and call the detector functions directly. No subprocess, no
# network, no PyYAML required for the core logic tests.
#
# Run:  python3 scripts/tests/test_feature_matrix_tiers.py
# (stdlib only; no pytest needed — also discoverable by `pytest`.)

from __future__ import annotations

import importlib.util
import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
DETECTOR = REPO_ROOT / "scripts" / "feature-matrix-tiers.py"


def _load_detector():
    """Import the detector module from its file path."""
    spec = importlib.util.spec_from_file_location("feature_matrix_tiers", DETECTOR)
    assert spec and spec.loader, "could not load {}".format(DETECTOR)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["feature_matrix_tiers"] = mod
    spec.loader.exec_module(mod)
    return mod


det = _load_detector()


# ---------------------------------------------------------------------------
# Helpers for building fixture crate directories
# ---------------------------------------------------------------------------

def _make_crate(tmpdir: str, src_files: dict = None, test_files: dict = None) -> Path:
    """Build a minimal fixture crate directory.

    src_files:  {relative_path_under_src: content, ...}
    test_files: {relative_path_under_tests: content, ...}

    Returns the crate root Path.
    """
    root = Path(tmpdir)
    # Minimal Cargo.toml so the fixture looks like a real crate
    (root / "Cargo.toml").write_text(
        '[package]\nname = "fixture"\nversion = "0.1.0"\n[features]\n',
        encoding="utf-8",
    )
    src_dir = root / "src"
    src_dir.mkdir()
    (src_dir / "lib.rs").write_text("", encoding="utf-8")

    if src_files:
        for rel, content in src_files.items():
            fpath = src_dir / rel
            fpath.parent.mkdir(parents=True, exist_ok=True)
            fpath.write_text(content, encoding="utf-8")

    if test_files:
        tests_dir = root / "tests"
        tests_dir.mkdir()
        for rel, content in test_files.items():
            fpath = tests_dir / rel
            fpath.parent.mkdir(parents=True, exist_ok=True)
            fpath.write_text(content, encoding="utf-8")

    return root


# ---------------------------------------------------------------------------
# cfg expression parser tests
# ---------------------------------------------------------------------------

class TestCfgParser(unittest.TestCase):
    """Unit tests for the recursive cfg expression parser."""

    def test_parse_feature(self):
        node = det.parse_cfg('feature = "foo"')
        self.assertIsInstance(node, det.CfgFeature)
        self.assertEqual(node.name, "foo")

    def test_parse_test(self):
        node = det.parse_cfg("test")
        self.assertIsInstance(node, det.CfgTest)

    def test_parse_not_feature(self):
        node = det.parse_cfg('not(feature = "foo")')
        self.assertIsInstance(node, det.CfgNot)
        self.assertIsInstance(node.inner, det.CfgFeature)
        self.assertEqual(node.inner.name, "foo")

    def test_parse_any(self):
        node = det.parse_cfg('any(feature = "a", feature = "b")')
        self.assertIsInstance(node, det.CfgAny)
        self.assertEqual(len(node.exprs), 2)

    def test_parse_all(self):
        node = det.parse_cfg('all(test, feature = "a")')
        self.assertIsInstance(node, det.CfgAll)
        self.assertEqual(len(node.exprs), 2)
        self.assertIsInstance(node.exprs[0], det.CfgTest)
        self.assertIsInstance(node.exprs[1], det.CfgFeature)

    def test_parse_nested_not_any(self):
        """cfg(not(any(target_arch = "wasm32", feature = "compact-index")))"""
        node = det.parse_cfg(
            'not(any(target_arch = "wasm32", feature = "compact-index"))'
        )
        self.assertIsInstance(node, det.CfgNot)
        self.assertIsInstance(node.inner, det.CfgAny)
        self.assertEqual(len(node.inner.exprs), 2)
        self.assertIsInstance(node.inner.exprs[1], det.CfgFeature)
        self.assertEqual(node.inner.exprs[1].name, "compact-index")

    def test_parse_other_predicate(self):
        node = det.parse_cfg('target_arch = "wasm32"')
        self.assertIsInstance(node, det.CfgOther)

    def test_parse_empty_raises(self):
        with self.assertRaises(det.ParseError):
            det.parse_cfg("")

    def test_parse_garbage_raises(self):
        """An unrecognised expression raises ParseError (fail-closed)."""
        with self.assertRaises(det.ParseError):
            det.parse_cfg("not(not(not(not())))")  # not() with empty inner

    def test_parse_unbalanced_parens_raises(self):
        with self.assertRaises(det.ParseError):
            det.parse_cfg('not(feature = "a"')

    def test_feature_under_not_direct(self):
        node = det.parse_cfg('not(feature = "foo")')
        self.assertTrue(det._feature_under_not(node, "foo"))
        self.assertFalse(det._feature_under_not(node, "bar"))

    def test_feature_under_not_nested(self):
        """cfg(not(any(target_arch = "wasm32", feature = "foo"))) => feature_under_not."""
        node = det.parse_cfg('not(any(target_arch = "wasm32", feature = "foo"))')
        self.assertTrue(det._feature_under_not(node, "foo"))

    def test_feature_under_not_false_for_plain_feature(self):
        """cfg(feature = "foo") alone does NOT put the feature under a not()."""
        node = det.parse_cfg('feature = "foo"')
        self.assertFalse(det._feature_under_not(node, "foo"))

    def test_feature_in_test_cfg(self):
        """cfg(all(test, feature = "foo")) => test + feature."""
        node = det.parse_cfg('all(test, feature = "foo")')
        self.assertTrue(det._feature_in_test_cfg(node, "foo"))
        self.assertFalse(det._feature_in_test_cfg(node, "bar"))

    def test_mentions_feature(self):
        node = det.parse_cfg('any(feature = "a", feature = "b")')
        self.assertTrue(det._mentions_feature(node, "a"))
        self.assertTrue(det._mentions_feature(node, "b"))
        self.assertFalse(det._mentions_feature(node, "c"))


# ---------------------------------------------------------------------------
# (a) seeded cfg(feature)-gated test in tests/ => SENSITIVE
# ---------------------------------------------------------------------------

class TestSensitiveTestFile(unittest.TestCase):
    """Acceptance criterion (a): a test file with cfg(feature) => SENSITIVE."""

    def test_test_file_gated_feature_sensitive(self):
        """tests/feature_gated.rs with #![cfg(feature = "my-feature")] => sensitive."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                test_files={
                    "feature_gated.rs": (
                        '#![cfg(feature = "my-feature")]\n'
                        "#[test]\n"
                        "fn it_works() { assert_eq!(1, 1); }\n"
                    )
                },
            )
            clf = det.classify_leg(root, ["my-feature"])
            self.assertTrue(
                clf.sensitive,
                "Expected SENSITIVE: test file has cfg(feature) gate. Reasons: {}".format(
                    clf.reasons
                ),
            )

    def test_benches_file_gated_feature_sensitive(self):
        """benches/*.rs with cfg(feature) => sensitive."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(tmp)
            benches_dir = root / "benches"
            benches_dir.mkdir()
            (benches_dir / "bench_foo.rs").write_text(
                '#[cfg(feature = "my-feature")]\nfn bench_it() {}\n',
                encoding="utf-8",
            )
            clf = det.classify_leg(root, ["my-feature"])
            self.assertTrue(clf.sensitive)

    def test_unrelated_feature_in_test_file_not_sensitive(self):
        """cfg(feature = "other") in test file does NOT make my-feature sensitive."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                test_files={
                    "other.rs": '#![cfg(feature = "other-feature")]\n#[test]\nfn t() {}\n'
                },
            )
            clf = det.classify_leg(root, ["my-feature"])
            self.assertFalse(
                clf.sensitive,
                "Expected NOT sensitive: cfg is for a different feature. Reasons: {}".format(
                    clf.reasons
                ),
            )


# ---------------------------------------------------------------------------
# (b) seeded cfg(not(feature)) in src/ => SENSITIVE
# ---------------------------------------------------------------------------

class TestSensitiveNotFeature(unittest.TestCase):
    """Acceptance criterion (b): cfg(not(feature)) in src/ => SENSITIVE."""

    def test_cfg_not_feature_in_src_sensitive(self):
        """#[cfg(not(feature = "my-feature"))] in src/lib.rs => sensitive."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                src_files={
                    "lib.rs": (
                        '#[cfg(not(feature = "my-feature"))]\n'
                        "fn fallback() {}\n"
                        '#[cfg(feature = "my-feature")]\n'
                        "fn fast_path() {}\n"
                    )
                },
            )
            clf = det.classify_leg(root, ["my-feature"])
            self.assertTrue(
                clf.sensitive,
                "Expected SENSITIVE: cfg(not(feature)) in src. Reasons: {}".format(
                    clf.reasons
                ),
            )

    def test_cfg_not_feature_via_cfg_attr_sensitive(self):
        """#[cfg_attr(not(feature = "my-feature"), allow(dead_code))] => sensitive."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                src_files={
                    "lib.rs": (
                        '#[cfg_attr(not(feature = "my-feature"), allow(dead_code))]\n'
                        "fn some_fn() {}\n"
                    )
                },
            )
            clf = det.classify_leg(root, ["my-feature"])
            self.assertTrue(clf.sensitive)


# ---------------------------------------------------------------------------
# (c) nested any()/all() combinators => SENSITIVE
# ---------------------------------------------------------------------------

class TestNestedCombinators(unittest.TestCase):
    """Acceptance criterion (c): nested any/all combinators parsed correctly."""

    def test_nested_not_any_sensitive(self):
        """cfg(not(any(target_arch = "wasm32", feature = "compact-index"))) => sensitive.

        This is the real sparq-core compact-index pattern from the design record.
        """
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                src_files={
                    "store.rs": (
                        '#[cfg(not(any(target_arch = "wasm32", feature = "compact-index")))]\n'
                        "type IndexSet = [(); 6];\n"
                        '#[cfg(any(target_arch = "wasm32", feature = "compact-index"))]\n'
                        "type IndexSet = [(); 3];\n"
                    )
                },
            )
            clf = det.classify_leg(root, ["compact-index"])
            self.assertTrue(
                clf.sensitive,
                "Expected SENSITIVE: nested not(any(wasm32, feature)) in src. "
                "Reasons: {}".format(clf.reasons),
            )

    def test_nested_all_with_not_sensitive(self):
        """cfg(all(not(feature = "foo"), bar)) => feature foo is under not()."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                src_files={
                    "lib.rs": (
                        '#[cfg(all(not(feature = "foo"), target_os = "linux"))]\n'
                        "fn linux_fallback() {}\n"
                    )
                },
            )
            clf = det.classify_leg(root, ["foo"])
            self.assertTrue(clf.sensitive)

    def test_plain_any_feature_in_test_file_sensitive(self):
        """tests/*.rs with cfg(any(feature = "a", feature = "b")) => sensitive for both."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                test_files={
                    "combo.rs": '#![cfg(any(feature = "a", feature = "b"))]\n#[test]\nfn t() {}\n'
                },
            )
            clf_a = det.classify_leg(root, ["a"])
            clf_b = det.classify_leg(root, ["b"])
            clf_c = det.classify_leg(root, ["c"])
            self.assertTrue(clf_a.sensitive, "feature a should be sensitive")
            self.assertTrue(clf_b.sensitive, "feature b should be sensitive")
            self.assertFalse(clf_c.sensitive, "feature c should not be sensitive")


# ---------------------------------------------------------------------------
# (d) parse error in cfg expression => SENSITIVE (fail-closed)
# ---------------------------------------------------------------------------

class TestParseErrorFailClosed(unittest.TestCase):
    """Acceptance criterion (d): a parse error classifies SENSITIVE (fail-closed)."""

    def test_unbalanced_paren_in_src_sensitive(self):
        """A file with #[cfg(not(feature = "foo")] (unbalanced) => sensitive.

        The detector encounters a parse error and must fail-closed.
        """
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                src_files={
                    # Deliberately malformed: missing closing paren before ']'
                    "broken.rs": '#[cfg(not(feature = "foo")]\nfn f() {}\n',
                },
            )
            clf = det.classify_leg(root, ["foo"])
            self.assertTrue(
                clf.sensitive,
                "Expected SENSITIVE on parse error (fail-closed). Got: {}".format(
                    clf.sensitive
                ),
            )
            self.assertIsNotNone(
                clf.error_msg,
                "Expected an error_msg when classification fails due to parse error.",
            )

    def test_empty_cfg_expression_sensitive(self):
        """A file with #[cfg()] (empty expression) => sensitive (fail-closed)."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                src_files={"lib.rs": "#[cfg()]\nfn f() {}\n"},
            )
            clf = det.classify_leg(root, ["any-feature"])
            self.assertTrue(clf.sensitive, "Empty cfg() should be fail-closed sensitive.")


# ---------------------------------------------------------------------------
# (e) unreadable crate dir => SENSITIVE (fail-closed)
# ---------------------------------------------------------------------------

class TestUnreadableDirFailClosed(unittest.TestCase):
    """Acceptance criterion (e): unreadable/missing dir => SENSITIVE (fail-closed)."""

    def test_missing_crate_dir_sensitive(self):
        """A non-existent crate directory => sensitive."""
        clf = det.classify_leg(Path("/nonexistent/does/not/exist"), ["my-feature"])
        self.assertTrue(clf.sensitive, "Missing dir must be fail-closed sensitive.")
        self.assertIsNotNone(clf.error_msg)

    def test_unreadable_tests_dir_sensitive(self):
        """An unreadable tests/ directory => sensitive (fail-closed)."""
        if os.geteuid() == 0:
            self.skipTest("running as root, cannot test permission denial")
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(tmp)
            tests_dir = root / "tests"
            tests_dir.mkdir()
            # Write a gated file, then revoke read permission on the directory
            (tests_dir / "secret.rs").write_text(
                '#![cfg(feature = "my-feature")]\n#[test]\nfn t() {}\n',
                encoding="utf-8",
            )
            os.chmod(str(tests_dir), 0o000)
            try:
                clf = det.classify_leg(root, ["my-feature"])
                self.assertTrue(
                    clf.sensitive,
                    "Unreadable tests/ dir must be fail-closed sensitive.",
                )
            finally:
                # Restore so TemporaryDirectory cleanup can delete it
                os.chmod(str(tests_dir), 0o755)


# ---------------------------------------------------------------------------
# (f) tier:check + sensitive => enforce exit != 0
# ---------------------------------------------------------------------------

class TestEnforceMode(unittest.TestCase):
    """Acceptance criterion (f): tier:check + sensitive verdict => enforce exit != 0."""

    def _make_legs(self, crate_dir: Path, tier: str = "check", tier_reason: str = "") -> list:
        """Build a minimal synthetic legs list for enforce testing."""
        leg: dict = {
            "name": "fixture (my-feature)",
            "crate": str(crate_dir),  # not used directly; we patch crate_dir below
            "features": "my-feature",
            "test": True,
            "tier": tier,
        }
        if tier_reason:
            leg["tier-reason"] = tier_reason
        return [leg]

    def _enforce_with_legs(self, legs: list) -> int:
        """Run cmd_enforce on a synthetic legs list with patched crate_dir."""
        return det.cmd_enforce(legs)

    def test_check_tier_sensitive_fails(self):
        """tier:check + sensitive detector verdict => enforce exit != 0."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                test_files={
                    "feature_gated.rs": '#![cfg(feature = "my-feature")]\n#[test]\nfn t() {}\n'
                },
            )
            # We call classify_leg directly to verify it's sensitive first
            clf = det.classify_leg(root, ["my-feature"])
            self.assertTrue(clf.sensitive, "Fixture must be sensitive for this test.")

            # Build a synthetic leg with tier:check pointing to our fixture crate
            # We need to call cmd_enforce with crate dir patched.
            # Use a subclass to inject the fixture crate_dir.
            original_crate_dir = det._crate_dir

            def patched_crate_dir(name: str) -> Path:
                if name == "fixture":
                    return root
                return original_crate_dir(name)

            det._crate_dir = patched_crate_dir
            try:
                legs = [
                    {
                        "name": "fixture (my-feature)",
                        "crate": "fixture",
                        "features": "my-feature",
                        "test": True,
                        "tier": "check",
                    }
                ]
                exit_code = det.cmd_enforce(legs)
                self.assertNotEqual(
                    exit_code,
                    0,
                    "tier:check + sensitive must cause enforce to exit non-zero.",
                )
            finally:
                det._crate_dir = original_crate_dir

    def test_check_tier_with_override_passes(self):
        """tier:check + sensitive + tier-reason override => enforce passes (exit 0)."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                test_files={
                    "feature_gated.rs": '#![cfg(feature = "my-feature")]\n#[test]\nfn t() {}\n'
                },
            )
            original_crate_dir = det._crate_dir

            def patched_crate_dir(name: str) -> Path:
                if name == "fixture":
                    return root
                return original_crate_dir(name)

            det._crate_dir = patched_crate_dir
            try:
                legs = [
                    {
                        "name": "fixture (my-feature)",
                        "crate": "fixture",
                        "features": "my-feature",
                        "test": True,
                        "tier": "check",
                        "tier-reason": "reviewed: tests run in default lane",
                    }
                ]
                exit_code = det.cmd_enforce(legs)
                self.assertEqual(
                    exit_code,
                    0,
                    "tier:check + sensitive with tier-reason override must pass.",
                )
            finally:
                det._crate_dir = original_crate_dir

    def test_sensitive_feature_no_test_leg_fails(self):
        """A sensitive feature appearing only in a test:false leg => sq-vya1 violation."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                test_files={
                    "feature_gated.rs": '#![cfg(feature = "my-feature")]\n#[test]\nfn t() {}\n'
                },
            )
            original_crate_dir = det._crate_dir

            def patched_crate_dir(name: str) -> Path:
                if name == "fixture":
                    return root
                return original_crate_dir(name)

            det._crate_dir = patched_crate_dir
            try:
                # test:false leg => sq-vya1 guard triggered
                legs = [
                    {
                        "name": "fixture (my-feature)",
                        "crate": "fixture",
                        "features": "my-feature",
                        "test": False,  # no test:true leg!
                        # no tier: field => defaults to 'test', fine for invariant 1
                    }
                ]
                exit_code = det.cmd_enforce(legs)
                self.assertNotEqual(
                    exit_code,
                    0,
                    "Sensitive feature with no test:true leg must fail enforce (sq-vya1).",
                )
            finally:
                det._crate_dir = original_crate_dir


# ---------------------------------------------------------------------------
# (j) [OPUS-5] cross-crate regression: cargo feature names are crate-LOCAL, so
# invariant (2) must key on (crate, feature). A test:true leg for feature F in
# crate A must NOT satisfy a sensitive F in crate B — keying on the bare name
# silently masked exactly that gap (15 feature names are shared across crates
# in the live fragments; sparq-arrow/arrow was standing in for sparq-py/arrow).
# ---------------------------------------------------------------------------

class TestCrossCrateFeatureNameCollision(unittest.TestCase):
    """Two crates share a feature name; only the UNRELATED crate has test:true."""

    def _run(self, extra_leg_fields: dict = None) -> int:
        """Build the two-crate fixture and return cmd_enforce's exit code.

        crate_a: NOT sensitive (no cfg refs at all), leg test:true.
        crate_b: SENSITIVE (feature-gated #[test] in tests/), leg test:false.
        Both legs declare the SAME feature name "shared-feat".
        """
        with tempfile.TemporaryDirectory() as tmp_a, \
                tempfile.TemporaryDirectory() as tmp_b:
            root_a = _make_crate(tmp_a)
            root_b = _make_crate(
                tmp_b,
                test_files={
                    "gated.rs": '#![cfg(feature = "shared-feat")]\n#[test]\nfn t() {}\n'
                },
            )

            # Precondition: the detector must disagree about the two crates,
            # or the test proves nothing.
            self.assertFalse(
                det.classify_leg(root_a, ["shared-feat"]).sensitive,
                "crate_a fixture must be NON-sensitive for this test.",
            )
            self.assertTrue(
                det.classify_leg(root_b, ["shared-feat"]).sensitive,
                "crate_b fixture must be SENSITIVE for this test.",
            )

            original_crate_dir = det._crate_dir

            def patched_crate_dir(name: str) -> Path:
                if name == "crate_a":
                    return root_a
                if name == "crate_b":
                    return root_b
                return original_crate_dir(name)

            det._crate_dir = patched_crate_dir
            try:
                leg_b = {
                    "name": "crate_b (shared-feat)",
                    "crate": "crate_b",
                    "features": "shared-feat",
                    "test": False,  # no cargo-test coverage for crate_b
                }
                leg_b.update(extra_leg_fields or {})
                legs = [
                    {
                        "name": "crate_a (shared-feat)",
                        "crate": "crate_a",
                        "features": "shared-feat",
                        "test": True,  # covers crate_a's feature, NOT crate_b's
                    },
                    leg_b,
                ]
                return det.cmd_enforce(legs)
            finally:
                det._crate_dir = original_crate_dir

    def test_same_name_other_crate_test_leg_does_not_satisfy_guard(self):
        """The unrelated crate's test:true leg must NOT cover crate_b's feature."""
        self.assertNotEqual(
            self._run(),
            0,
            "A test:true leg for the SAME feature name in a DIFFERENT crate must "
            "not satisfy the sq-vya1 guard — feature names are crate-local.",
        )

    def test_written_test_reason_exempts_the_leg(self):
        """A written test-reason: on the leg is the reviewed escape hatch."""
        self.assertEqual(
            self._run({"test-reason": "coverage lives in the python.yml pytest job"}),
            0,
            "A test:false leg carrying a written test-reason: must pass invariant (2).",
        )

    def test_blank_test_reason_does_not_exempt_the_leg(self):
        """An empty/whitespace test-reason: is not a justification."""
        self.assertNotEqual(
            self._run({"test-reason": "   "}),
            0,
            "A blank test-reason: must not silence the sq-vya1 guard.",
        )


# ---------------------------------------------------------------------------
# (g) mutation check: fail-open copy => non-sensitive on error
# ---------------------------------------------------------------------------

class TestMutationFailOpen(unittest.TestCase):
    """Acceptance criterion (g): the fail-open mutation must disagree with fail-closed.

    This proves the fail-closed behaviour is load-bearing: if someone
    flips the detector to fail-open on errors, a test goes red.
    """

    def test_fail_open_returns_non_sensitive_on_missing_dir(self):
        """Mutant (fail_open=True) returns non-sensitive on a missing crate dir."""
        clf = det.classify_leg(
            Path("/nonexistent/does/not/exist"), ["my-feature"], fail_open=True
        )
        self.assertFalse(
            clf.sensitive,
            "Mutant (fail_open=True) must return non-sensitive on error "
            "(proving the fail-closed path is load-bearing).",
        )

    def test_fail_closed_returns_sensitive_on_missing_dir(self):
        """The real detector (fail_open=False) returns sensitive on same error."""
        clf = det.classify_leg(
            Path("/nonexistent/does/not/exist"), ["my-feature"], fail_open=False
        )
        self.assertTrue(
            clf.sensitive,
            "Real detector (fail-closed) must return sensitive on missing dir.",
        )

    def test_mutation_check_divergence(self):
        """Fail-open and fail-closed MUST give different answers on error cases.

        This is the canonical mutation check: if someone changes fail-closed
        to fail-open, tests (b) and (e) above would pass but THIS test would
        fail — making the regression visible.
        """
        missing = Path("/nonexistent/crate/path")
        clf_real = det.classify_leg(missing, ["feat"])
        clf_mutant = det.classify_leg(missing, ["feat"], fail_open=True)
        self.assertNotEqual(
            clf_real.sensitive,
            clf_mutant.sensitive,
            "fail-closed (sensitive=True) must differ from fail-open (sensitive=False) "
            "on an error case. Both returned sensitive={}.".format(clf_real.sensitive),
        )

    def test_fail_open_non_sensitive_on_unreadable_tests_dir(self):
        """Mutant (fail_open=True) returns non-sensitive on unreadable tests/ dir."""
        if os.geteuid() == 0:
            self.skipTest("running as root, cannot test permission denial")
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(tmp)
            tests_dir = root / "tests"
            tests_dir.mkdir()
            (tests_dir / "gated.rs").write_text(
                '#![cfg(feature = "feat")]\n#[test]\nfn t() {}\n',
                encoding="utf-8",
            )
            os.chmod(str(tests_dir), 0o000)
            try:
                clf_open = det.classify_leg(root, ["feat"], fail_open=True)
                clf_closed = det.classify_leg(root, ["feat"], fail_open=False)
                self.assertFalse(clf_open.sensitive, "fail-open must not escalate on error")
                self.assertTrue(clf_closed.sensitive, "fail-closed must escalate on error")
            finally:
                os.chmod(str(tests_dir), 0o755)


# ---------------------------------------------------------------------------
# (h) non-sensitive fixture => NOT sensitive
# ---------------------------------------------------------------------------

class TestNonSensitive(unittest.TestCase):
    """Acceptance criterion (h): a crate with no cfg refs is NOT sensitive."""

    def test_empty_crate_not_sensitive(self):
        """A crate with no cfg(feature) references is not sensitive."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                src_files={"lib.rs": "pub fn add(a: i32, b: i32) -> i32 { a + b }\n"},
            )
            clf = det.classify_leg(root, ["my-feature"])
            self.assertFalse(
                clf.sensitive,
                "Expected NOT sensitive: no cfg refs. Reasons: {}".format(clf.reasons),
            )

    def test_cfg_for_other_feature_not_sensitive(self):
        """cfg(feature = "other") does NOT make "my-feature" sensitive."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                src_files={
                    "lib.rs": (
                        '#[cfg(feature = "other")]\n'
                        "fn other_path() {}\n"
                    )
                },
            )
            clf = det.classify_leg(root, ["my-feature"])
            self.assertFalse(clf.sensitive)


# ---------------------------------------------------------------------------
# (i) cfg(all(test, feature)) in src/ => SENSITIVE (b-direct pattern)
# ---------------------------------------------------------------------------

class TestBDirectPattern(unittest.TestCase):
    """Acceptance criterion (i): cfg(all(test, feature)) in src/ => SENSITIVE."""

    def test_all_test_feature_in_src_sensitive(self):
        """#[cfg(all(test, feature = "my-feature"))] in src/lib.rs => sensitive."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                src_files={
                    "lib.rs": (
                        '#[cfg(all(test, feature = "my-feature"))]\n'
                        "mod tests {\n"
                        "    #[test]\n"
                        "    fn it_works() {}\n"
                        "}\n"
                    )
                },
            )
            clf = det.classify_leg(root, ["my-feature"])
            self.assertTrue(
                clf.sensitive,
                "Expected SENSITIVE: cfg(all(test, feature)) in src. Reasons: {}".format(
                    clf.reasons
                ),
            )

    def test_all_feature_test_in_src_sensitive(self):
        """cfg(all(feature = "my-feature", test)) — reversed order — also sensitive."""
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                src_files={
                    "lib.rs": (
                        '#[cfg(all(feature = "my-feature", test))]\n'
                        "mod tests {\n"
                        "    #[test]\n"
                        "    fn it_works() {}\n"
                        "}\n"
                    )
                },
            )
            clf = det.classify_leg(root, ["my-feature"])
            self.assertTrue(clf.sensitive)


# ---------------------------------------------------------------------------
# cfg expression extraction tests
# ---------------------------------------------------------------------------

class TestCfgExtraction(unittest.TestCase):
    """Tests for extracting cfg expressions from Rust source content."""

    def test_extract_single_cfg(self):
        content = (
            '#[cfg(feature = "vector-r")]\n'
            'fn a() {}\n'
            '#[cfg(not(feature = "quoted-triples"))]\n'
            'fn g() {}\n'
        )
        exprs = det._extract_cfg_expressions(content)
        self.assertEqual(
            exprs,
            ['feature = "vector-r"', 'not(feature = "quoted-triples")'],
        )

    def test_extract_inner_cfg(self):
        content = '#![cfg(feature = "foo")]\nfn f() {}\n'
        exprs = det._extract_cfg_expressions(content)
        self.assertEqual(exprs, ['feature = "foo"'])

    def test_extract_cfg_attr(self):
        content = '#[cfg_attr(not(feature = "foo"), allow(dead_code))]\nfn f() {}\n'
        exprs = det._extract_cfg_expressions(content)
        self.assertEqual(exprs, ['not(feature = "foo")'])

    def test_extract_multiple(self):
        content = (
            '#[cfg(feature = "a")]\nfn a() {}\n'
            '#[cfg(not(feature = "b"))]\nfn b() {}\n'
        )
        exprs = det._extract_cfg_expressions(content)
        self.assertEqual(len(exprs), 2)
        self.assertIn('feature = "a"', exprs)
        self.assertIn('not(feature = "b")', exprs)

    def test_extract_nested(self):
        content = '#[cfg(not(any(target_arch = "wasm32", feature = "ci")))]\nfn f() {}\n'
        exprs = det._extract_cfg_expressions(content)
        self.assertEqual(len(exprs), 1)
        self.assertIn('not(any(target_arch = "wasm32", feature = "ci"))', exprs)

    def test_sanitizer_preserves_lines_and_live_cfg_after_comment_quote(self):
        # [SONNET-4.6] Regression: a quote in prose must not erase later attributes.
        content = (
            '// prose mentions "an unterminated literal\n'
            '#[cfg(not(feature = "quoted-triples"))]\n'
            'fn materialize_default() {}\n'
        )
        safe = det._sanitize_for_cfg_scan(content)
        self.assertEqual(safe.count("\n"), content.count("\n"))
        self.assertEqual(len(safe), len(content))
        self.assertEqual(
            det._extract_cfg_expressions(content),
            ['not(feature = "quoted-triples")'],
        )

    def test_multiline_raw_string_decoy_preserves_live_cfg(self):
        content = (
            'const DOC: &str = r##"#[cfg(not(feature = "window-origin"))]\n'
            'still a string"##;\n'
            '#[cfg(all(test, feature = "window-origin"))]\n'
            'mod tests {}\n'
        )
        safe = det._sanitize_for_cfg_scan(content)
        self.assertEqual(safe.count("\n"), content.count("\n"))
        self.assertEqual(len(safe), len(content))
        self.assertEqual(
            det._extract_cfg_expressions(content),
            ['all(test, feature = "window-origin")'],
        )

    def test_escaped_cr_does_not_mask_following_live_cfg(self):
        content = (
            'let sep = "\\r";\n'
            '#[cfg(not(feature = "quoted-triples"))]\n'
            'fn materialize_default() {}\n'
        )
        self.assertEqual(
            det._extract_cfg_expressions(content),
            ['not(feature = "quoted-triples")'],
        )

    def test_ordinary_literal_trailing_r_does_not_open_raw_string(self):
        for literal in ('"mode r"', '"w/r"'):
            with self.subTest(literal=literal):
                content = (
                    'let mode = {};\n'.format(literal)
                    + '#[cfg(not(feature = "quoted-triples"))]\n'
                    + 'fn g() {}\n'
                )
                self.assertEqual(
                    det._extract_cfg_expressions(content),
                    ['not(feature = "quoted-triples")'],
                )

    def test_nested_block_comment_cfg_is_masked(self):
        content = (
            '/* outer /* #[cfg(feature = "decoy")] */ comment */\n'
            '#[cfg(feature = "live")]\n'
        )
        self.assertEqual(det._extract_cfg_expressions(content), ['feature = "live"'])

    def test_regression_cfgs_from_rsp_and_reason_sources(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_crate(
                tmp,
                src_files={
                    "lib.rs": (
                        '// RSP source prose with an unmatched " quote\n'
                        '#[cfg(all(test, feature = "window-origin"))]\n'
                        'mod window_origin_tests {}\n'
                        '#[cfg(all(test, not(feature = "window-origin")))]\n'
                        'mod window_origin_default_tests {}\n'
                        '#[cfg(feature = "quoted-triples")]\n'
                        'fn reified() {}\n'
                        '#[cfg(not(feature = "quoted-triples"))]\n'
                        'fn materialized() {}\n'
                    )
                },
            )
            rsp = det.classify_leg(root, ["window-origin"])
            reason = det.classify_leg(root, ["quoted-triples"])
            self.assertTrue(rsp.sensitive)
            self.assertTrue(reason.sensitive)


# ---------------------------------------------------------------------------
# Utility tests
# ---------------------------------------------------------------------------

class TestSplitCfgArgs(unittest.TestCase):
    def test_simple(self):
        result = det._split_cfg_args('feature = "a", feature = "b"')
        self.assertEqual(result, ['feature = "a"', 'feature = "b"'])

    def test_nested(self):
        result = det._split_cfg_args('any(a, b), feature = "c"')
        self.assertEqual(result, ["any(a, b)", 'feature = "c"'])

    def test_single(self):
        result = det._split_cfg_args('feature = "foo"')
        self.assertEqual(result, ['feature = "foo"'])

    def test_empty(self):
        result = det._split_cfg_args("")
        self.assertEqual(result, [])


# ---------------------------------------------------------------------------
# Integration: features_from_leg helper
# ---------------------------------------------------------------------------

class TestFeaturesFromLeg(unittest.TestCase):
    def test_single_feature(self):
        leg = {"features": "foo"}
        self.assertEqual(det._features_from_leg(leg), ["foo"])

    def test_multiple_features(self):
        leg = {"features": "foo,bar, baz"}
        self.assertEqual(det._features_from_leg(leg), ["foo", "bar", "baz"])

    def test_empty_features(self):
        leg = {"features": ""}
        self.assertEqual(det._features_from_leg(leg), [])


# ---------------------------------------------------------------------------
# Runner
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    unittest.main()
