#!/usr/bin/env python3
# [OPUS-5] Hermetic tests for scripts/preflight.py — the diff-scoped pre-submit
# gate runner.
#
# EVERY test here is NAMED for the guard it pins and is written to go RED when
# that guard is deleted or inverted. The mutation log in the PR body records the
# measured kill for each. Follows scripts/tests/test_gates.py: stdlib only, no
# pytest required, no network; the two end-to-end cases build a throwaway git tree
# under tempfile rather than touching the real repo.
#
# Run:  python3 scripts/tests/test_preflight.py

from __future__ import annotations

import importlib.util
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, REPO_ROOT / "scripts" / filename)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


pf = _load("preflight", "preflight.py")

RUST = "crates/sparq-x/src/lib.rs"
PY = "scripts/thing.py"


class GuardShapeDetection(unittest.TestCase):
    """`added_symbols` must recognise a guard-shaped SURFACE and nothing else."""

    def test_pub_fn_with_guard_stem_is_detected(self) -> None:
        # Kills: deleting the `validate` stem from GUARD_STEMS.
        self.assertEqual(
            pf.added_symbols({RUST: ["pub fn validate_envelope(e: &E) -> bool {"]}),
            [(RUST, "validate_envelope")],
        )

    def test_private_rust_fn_is_not_a_surface(self) -> None:
        # Kills: dropping the `pub` requirement from RUST_GUARD_RE — the check
        # would then fire on every private helper and workers would disable it.
        self.assertEqual(pf.added_symbols({RUST: ["fn check_bounds(n: usize) {"]}), [])

    def test_pub_fn_without_a_guard_stem_is_not_flagged(self) -> None:
        # Kills: widening RUST_GUARD_RE to any `pub fn`.
        self.assertEqual(pf.added_symbols({RUST: ["pub fn render_row(r: &R) {"]}), [])

    def test_nested_python_def_is_not_a_surface(self) -> None:
        # Kills: relaxing PY_GUARD_RE's column-0 anchor.
        self.assertEqual(pf.added_symbols({PY: ["    def validate_lease(x):"]}), [])

    def test_module_level_python_def_is_a_surface(self) -> None:
        self.assertEqual(
            pf.added_symbols({PY: ["def validate_lease(x):"]}), [(PY, "validate_lease")]
        )

    def test_paths_outside_crate_src_and_scripts_are_out_of_scope(self) -> None:
        # Kills: dropping the RUST_SRC_RE / PY_SCRIPT_RE path filter.
        self.assertEqual(
            pf.added_symbols({"docs/x.rs": ["pub fn validate_envelope() {"]}), []
        )


class SuppressionRequiresAReason(unittest.TestCase):
    """The escape hatch's whole value is the recorded reason."""

    def test_marker_with_a_reason_suppresses(self) -> None:
        self.assertEqual(
            pf.added_symbols(
                {RUST: [
                    "// preflight-allow: guard-untested — proven by the kani harness",
                    "pub fn validate_envelope() {",
                ]}
            ),
            [],
        )

    def test_marker_without_a_reason_does_not_suppress(self) -> None:
        # Kills: dropping the `and m.group("why").strip()` conjunct in suppressed().
        # Without it a bare `preflight-allow: guard-untested —` silences the check,
        # which is exactly the fail-open the corpus is full of.
        self.assertEqual(
            pf.added_symbols(
                {RUST: ["// preflight-allow: guard-untested —", "pub fn validate_envelope() {"]}
            ),
            [(RUST, "validate_envelope")],
        )

    def test_marker_for_a_different_check_does_not_suppress(self) -> None:
        # Kills: ignoring the check name in SUPPRESS_RE.
        self.assertEqual(
            pf.added_symbols(
                {RUST: [
                    "// preflight-allow: no-perf-numbers — unrelated",
                    "pub fn validate_envelope() {",
                ]}
            ),
            [(RUST, "validate_envelope")],
        )

    def test_a_hyphenated_check_name_parses_whole(self) -> None:
        # REGRESSION (mutation M3). SUPPRESS_RE originally allowed the separator to
        # be a bare `-`, so `[a-z0-9-]+` backtracked and ate its own hyphen:
        # `preflight-allow: guard-untested —` parsed as check=`guard`,
        # why=`untested —`. A marker with NO reason therefore produced a
        # well-formed match for a check name nobody wrote. Requiring whitespace on
        # BOTH sides of the separator is what makes this parse correctly — invert
        # that and this test reds.
        m = pf.SUPPRESS_RE.search("// preflight-allow: guard-untested — real reason")
        self.assertIsNotNone(m)
        assert m is not None
        self.assertEqual(m.group("check"), "guard-untested")
        self.assertEqual(m.group("why"), "real reason")

    def test_a_reasonless_marker_does_not_match_at_all(self) -> None:
        # The reason is mandatory in the REGEX, not in a downstream conjunct. If
        # this ever matches, `suppressed()` has a fail-open escape hatch.
        self.assertIsNone(pf.SUPPRESS_RE.search("// preflight-allow: guard-untested —"))
        self.assertIsNone(pf.SUPPRESS_RE.search("// preflight-allow: guard-untested —   "))

    def test_marker_two_lines_above_does_not_suppress(self) -> None:
        # Kills: widening the suppression window past (idx, idx-1), which would let
        # an unrelated marker elsewhere in the hunk silence a guard.
        self.assertEqual(
            pf.added_symbols(
                {RUST: [
                    "// preflight-allow: guard-untested — stale marker",
                    "",
                    "pub fn validate_envelope() {",
                ]}
            ),
            [(RUST, "validate_envelope")],
        )


class AddedLineParsing(unittest.TestCase):
    def test_only_added_lines_are_read(self) -> None:
        # Kills: reading removed or context lines — a guard DELETED by the diff
        # would otherwise be reported as newly untested.
        diff = (
            "diff --git a/scripts/a.py b/scripts/a.py\n"
            "--- a/scripts/a.py\n"
            "+++ b/scripts/a.py\n"
            "@@ -1,0 +2 @@\n"
            "+def validate_x(v):\n"
            "-def validate_gone(v):\n"
            " def context_untouched(v):\n"
        )
        self.assertEqual(pf.parse_added(diff), {"scripts/a.py": ["def validate_x(v):"]})

    def test_the_plus_plus_plus_header_is_not_treated_as_content(self) -> None:
        diff = "--- a/scripts/a.py\n+++ b/scripts/a.py\n@@ -0,0 +1 @@\n+def validate_x(v):\n"
        self.assertNotIn("+ b/scripts/a.py", pf.parse_added(diff)["scripts/a.py"])


class _Tree:
    """A throwaway git tree so check_guard_untested's `git ls-files` has something."""

    def __init__(self, td: str) -> None:
        self.root = Path(td)
        (self.root / "crates/sparq-x/src").mkdir(parents=True)
        (self.root / "crates/sparq-x/src/lib.rs").write_text("pub fn validate_envelope() {}\n")
        subprocess.run(["git", "init", "-q"], cwd=self.root, check=True)
        self.stage()

    def stage(self) -> None:
        subprocess.run(["git", "add", "-A"], cwd=self.root, check=True)

    def write(self, rel: str, text: str) -> None:
        p = self.root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)
        self.stage()


ADDED = {"crates/sparq-x/src/lib.rs": ["pub fn validate_envelope() {}"]}


class EndToEndGuardUntested(unittest.TestCase):
    """The behaviour the check exists for, exercised through the real entry point."""

    def test_guard_with_no_test_anywhere_reds(self) -> None:
        # THE headline guard. Kills: making check_guard_untested return [] always.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            self.assertEqual(len(pf.check_guard_untested(ADDED, t.root)), 1)

    def test_guard_named_by_an_integration_test_passes(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/tests/t.rs",
                    "#[test]\nfn t() { sparq_x::validate_envelope(); }\n")
            self.assertEqual(pf.check_guard_untested(ADDED, t.root), [])

    def test_an_unrelated_test_does_not_satisfy_the_guard(self) -> None:
        # THE anti-vacuity case. If this passed, any test file anywhere in the repo
        # would satisfy every guard and the check would be decorative.
        # Kills: replacing the per-symbol word search with "is there any test file".
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/tests/t.rs",
                    "#[test]\nfn t() { sparq_x::something_else(); }\n")
            self.assertEqual(len(pf.check_guard_untested(ADDED, t.root)), 1)

    def test_a_substring_match_does_not_satisfy_the_guard(self) -> None:
        # Kills: dropping the \b word boundaries — `validate_envelope_legacy` in an
        # old test would otherwise silently satisfy a new `validate_envelope`.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/tests/t.rs",
                    "#[test]\nfn t() { sparq_x::validate_envelope_legacy(); }\n")
            self.assertEqual(len(pf.check_guard_untested(ADDED, t.root)), 1)

    def test_an_in_file_cfg_test_module_satisfies_the_guard(self) -> None:
        # Kills: restricting the corpus to tests/ directories only — a Rust unit
        # test legitimately lives in the same file as the guard.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/lib.rs",
                    "pub fn validate_envelope() {}\n"
                    "#[cfg(test)]\nmod tests {\n"
                    "  #[test] fn t() { super::validate_envelope(); }\n}\n")
            self.assertEqual(pf.check_guard_untested(ADDED, t.root), [])

    def test_a_python_in_file_self_test_block_satisfies_the_guard(self) -> None:
        # REGRESSION: measured false positive on the real tree. This repo's
        # dominant convention for scripts/*.py is an in-file `--self-test` block,
        # not a tests/ file. scripts/release-interval-guard.py::check_version_group
        # is exercised only there, and the first version of this check flagged it.
        # Delete the is_py_script self-test branch in test_corpus() and this reds.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("scripts/thing.py",
                    "def validate_lease(x):\n    return x\n"
                    "def self_test():\n    assert validate_lease(1) == 1\n")
            added = {"scripts/thing.py": ["def validate_lease(x):"]}
            self.assertEqual(pf.check_guard_untested(added, t.root), [])

    def test_a_python_script_with_no_self_test_does_not_satisfy_the_guard(self) -> None:
        # The other direction: a plain script that merely CALLS the guard is not a
        # test host. Without this, every call site would satisfy every guard.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("scripts/thing.py", "def validate_lease(x):\n    return x\n")
            t.write("scripts/caller.py", "import thing\nthing.validate_lease(1)\n")
            added = {"scripts/thing.py": ["def validate_lease(x):"]}
            self.assertEqual(len(pf.check_guard_untested(added, t.root)), 1)

    def test_a_non_test_rust_src_file_naming_the_guard_does_not_satisfy_it(self) -> None:
        # Kills: dropping the `#[cfg(test)]` requirement on in-src corpus entries —
        # an ordinary CALL SITE would then count as a test.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/caller.rs",
                    "pub fn go() { crate::validate_envelope(); }\n")
            self.assertEqual(len(pf.check_guard_untested(ADDED, t.root)), 1)


class ADefiningFileIsNotItsOwnTest(unittest.TestCase):
    """REGRESSION — the checker committed the defect it exists to prevent.

    The first version admitted a whole file on a FILE-LEVEL marker (`#[cfg(test)]`
    anywhere in a Rust src file, `--self-test` anywhere in a scripts/*.py) and then
    word-searched that file's ENTIRE text — which contains the `pub fn` / `def`
    definition line itself. So an empty `mod tests {}` made every guard in the file
    permanently invisible: 48 of 191 guard-shaped surface symbols in this tree could
    never be reported. Every test in this class RED on the pre-fix code.
    """

    def test_an_empty_cfg_test_module_does_not_satisfy_the_guard(self) -> None:
        # THE headline case. Pre-fix: 0 findings. Post-fix: 1.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/lib.rs",
                    "pub fn validate_envelope() {}\n#[cfg(test)]\nmod tests {}\n")
            self.assertEqual(len(pf.check_guard_untested(ADDED, t.root)), 1)

    def test_a_cfg_test_module_that_never_names_the_guard_does_not_satisfy_it(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/lib.rs",
                    "pub fn validate_envelope() {}\n"
                    "#[cfg(test)]\nmod tests {\n  #[test] fn t() { assert!(true); }\n}\n")
            self.assertEqual(len(pf.check_guard_untested(ADDED, t.root)), 1)

    def test_a_call_site_beside_a_test_module_does_not_satisfy_the_guard(self) -> None:
        # The subtler shape: the file HAS a real test module, but the only mention
        # of this guard is production code. Excluding just the definition LINE
        # would let this through; scoping to the test region does not.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/lib.rs",
                    "pub fn validate_envelope() {}\n"
                    "pub fn go() { validate_envelope(); }\n"
                    "#[cfg(test)]\nmod tests {\n  #[test] fn t() { super::go(); }\n}\n")
            self.assertEqual(len(pf.check_guard_untested(ADDED, t.root)), 1)

    def test_a_python_self_test_that_never_names_the_guard_does_not_satisfy_it(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("scripts/thing.py",
                    "def validate_lease(x):\n    return x\n"
                    "def self_test():\n    assert True\n")
            added = {"scripts/thing.py": ["def validate_lease(x):"]}
            self.assertEqual(len(pf.check_guard_untested(added, t.root)), 1)

    def test_a_bare_self_test_flag_string_is_not_a_test_region(self) -> None:
        # Pre-fix, SELFTEST_MARKER_RE admitted the file on the literal
        # `--self-test` appearing ANYWHERE — an argparse help string was enough.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("scripts/thing.py",
                    "def validate_lease(x):\n    return x\n"
                    'ap.add_argument("--self-test", help="run the self-test")\n')
            added = {"scripts/thing.py": ["def validate_lease(x):"]}
            self.assertEqual(len(pf.check_guard_untested(added, t.root)), 1)

    def test_cfg_not_test_is_not_a_test_region(self) -> None:
        # `#[cfg(not(test))]` is compiled in ORDINARY builds. Accepting any cfg
        # whose text merely contains `test` would make this a test region.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/lib.rs",
                    "pub fn validate_envelope() {}\n"
                    "#[cfg(not(test))]\nmod prod {\n  fn p() { super::validate_envelope(); }\n}\n")
            self.assertEqual(len(pf.check_guard_untested(ADDED, t.root)), 1)

    def test_cfg_any_with_a_non_test_disjunct_is_not_a_test_region(self) -> None:
        # `any(not(feature = "x"), test)` holds in a plain build with the feature
        # off — production code, not a test.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/lib.rs",
                    "pub fn validate_envelope() {}\n"
                    '#[cfg(any(not(feature = "odrl-bridge"), test))]\n'
                    "mod shim {\n  fn p() { super::validate_envelope(); }\n}\n")
            self.assertEqual(len(pf.check_guard_untested(ADDED, t.root)), 1)

    def test_cfg_all_test_and_a_feature_IS_a_test_region(self) -> None:
        # The other direction — `all(test, feature = "x")` only ever compiles under
        # test, so it must still count. Inverting _cfg_implies_test reds this.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/lib.rs",
                    "pub fn validate_envelope() {}\n"
                    '#[cfg(all(test, feature = "service"))]\n'
                    "mod tests {\n  #[test] fn t() { super::validate_envelope(); }\n}\n")
            self.assertEqual(pf.check_guard_untested(ADDED, t.root), [])

    def test_a_doctest_naming_the_guard_satisfies_it(self) -> None:
        # `cargo test` runs doctests and they stop compiling if the item is
        # deleted, so a fenced doc example is a real test.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/lib.rs",
                    "/// Checks an envelope.\n"
                    "/// ```\n"
                    "/// assert!(sparq_x::validate_envelope());\n"
                    "/// ```\n"
                    "pub fn validate_envelope() -> bool { true }\n")
            self.assertEqual(pf.check_guard_untested(ADDED, t.root), [])

    def test_a_fuzz_target_naming_the_guard_satisfies_it(self) -> None:
        # CORRECTION (round 3). This comment used to read:
        #
        #   "MEASURED: the one false positive among the 39 distinct symbols this
        #    check reports on the real tree."
        #
        # That was false. Deleting each of the then-32 Python firings and running
        # that script's own gating `--self-test` reds 10 of them, and an 11th —
        # `sparq-mpc::check_bounded_path` — was a Rust separate-file test module
        # this resolver could not see (now fixed; see SeparateFileTestModules).
        # It also named the WRONG SYMBOL. `eval::validate_graph` is `pub(crate)`,
        # and `fuzz/` is a separate cargo workspace that cannot reference a
        # crate-private item — so it can never "stop compiling if the function is
        # deleted". It appears in validate_shacl.rs only in a header comment.
        # What survives is what this test actually pins, and it is real: a fuzz
        # target that CALLS a public guard is compiled by the fuzz runner and
        # breaks when the guard is deleted. Measured over all 14 fuzz/**.rs and
        # all 148 guard-shaped names, with mask_rust separating code from prose:
        # `validate` matches in CODE (that is what /fuzz/ genuinely pins), while
        # `validate_graph`, `guard` and `required` match only in comment/string
        # text — the known whole-file-host limitation recorded in preflight.py.
        # Kills: dropping "/fuzz/" from TEST_PATH_HINTS.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("fuzz/fuzz_targets/f.rs",
                    "fuzz_target!(|d: &[u8]| { let _ = sparq_x::validate_envelope(); });\n")
            self.assertEqual(pf.check_guard_untested(ADDED, t.root), [])

    def test_an_example_naming_the_guard_satisfies_it(self) -> None:
        # MEASURED false positive, round 3: `sparq-shacl::validate_with_model` is
        # named by two `crates/sparq-shacl/examples/*.rs` and by no test. `cargo
        # test` BUILDS examples, so deleting the function fails `cargo test` —
        # the identical argument already accepted for /fuzz/ and /benches/, and
        # strictly stronger (fuzz targets are a separate workspace that `cargo
        # test` does not build). Kills: dropping "/examples/" from
        # TEST_PATH_HINTS.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/examples/demo.rs",
                    "fn main() { sparq_x::validate_envelope(); }\n")
            self.assertEqual(pf.check_guard_untested(ADDED, t.root), [])

    def test_doc_PROSE_naming_the_guard_does_not_satisfy_it(self) -> None:
        # Only the fenced code counts. Prose is documentation, not a test — and
        # accepting it would let a diff test its guard by describing it.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/lib.rs",
                    "/// See `validate_envelope` for the envelope rules.\n"
                    "pub fn validate_envelope() -> bool { true }\n")
            self.assertEqual(len(pf.check_guard_untested(ADDED, t.root)), 1)


class SeparateFileTestModules(unittest.TestCase):
    """REGRESSION (round-3 blocking) — `#[cfg(test)] mod X;` puts the test text in
    ANOTHER FILE, and the resolver could not see it.

    The gating attribute lives in the PARENT, so `X.rs` contains no `#[cfg(test)]`
    of its own and contributed ZERO test text. `TEST_PATH_HINTS` did not cover it
    either: it matches the DIRECTORY `/tests/`, and these files are
    `planner/tests.rs`, `adversarial_tests.rs`, `ttl/tests.rs`. Measured on the
    real tree: 14 such files (13 under a bare `#[cfg(test)]`, plus
    `sparq-engine/src/cs_gate.rs` under `all(test, feature = "cs-planner")`),
    14 of 14 invisible — which is why
    `sparq-mpc::check_bounded_path`, called by 14 `#[test]` fns in
    `hidden_path/planner/tests.rs`, was reported as named by no test.

    This is the SAME defect class as round 1 (test text that exists but is not
    attributed to the guard), so these tests pin the class, not one instance:
    the sibling form, the nested form, the `mod.rs` form, the transitive form,
    and both directions of the cfg predicate.
    """

    GUARD_IN_TESTS = "#[test]\nfn t() { crate::validate_envelope(); }\n"

    def test_a_sibling_cfg_test_module_file_satisfies_the_guard(self) -> None:
        # THE headline case. Kills: rust_cfg_test_mod_decls returning [], and
        # dropping the `rel in test_module_files` branch of searchable_test_text.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/lib.rs",
                    "pub fn validate_envelope() {}\n#[cfg(test)]\nmod unit;\n")
            t.write("crates/sparq-x/src/unit.rs", self.GUARD_IN_TESTS)
            self.assertEqual(pf.check_guard_untested(ADDED, t.root), [])

    def test_a_nested_cfg_test_module_file_satisfies_the_guard(self) -> None:
        # The real shape on this tree: `planner.rs` declares it, the body is in
        # `planner/tests.rs`. Kills: resolving `mod X;` against the wrong
        # directory (the declaring file's own dir instead of its module dir).
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/planner.rs", "#[cfg(test)]\nmod unit;\n")
            t.write("crates/sparq-x/src/planner/unit.rs", self.GUARD_IN_TESTS)
            self.assertEqual(pf.check_guard_untested(ADDED, t.root), [])

    def test_a_cfg_test_module_resolved_through_mod_rs_satisfies_the_guard(self) -> None:
        # Kills: resolving only `X.rs` and not `X/mod.rs`.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/lib.rs",
                    "pub fn validate_envelope() {}\n#[cfg(test)]\npub mod suite;\n")
            t.write("crates/sparq-x/src/suite/mod.rs", self.GUARD_IN_TESTS)
            self.assertEqual(pf.check_guard_untested(ADDED, t.root), [])

    def test_a_module_declared_by_a_test_only_module_is_also_test_text(self) -> None:
        # Transitive closure: a module reached only from a test-only module is
        # itself compiled only under test, with no cfg attribute of its own.
        # Kills: seeding the set but never closing it.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/lib.rs",
                    "pub fn validate_envelope() {}\n#[cfg(test)]\nmod unit;\n")
            t.write("crates/sparq-x/src/unit.rs", "mod fixtures;\n")
            t.write("crates/sparq-x/src/unit/fixtures.rs", self.GUARD_IN_TESTS)
            self.assertEqual(pf.check_guard_untested(ADDED, t.root), [])

    def test_a_plain_mod_declaration_is_not_a_test_module(self) -> None:
        # THE anti-vacuity direction, and the one that keeps round-1 shut. An
        # ORDINARY `mod helpers;` is production code; admitting its whole text
        # would make every same-crate call site satisfy every guard.
        # Kills: seeding rust_test_module_files from rust_mod_decls instead of
        # rust_cfg_test_mod_decls.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/lib.rs",
                    "pub fn validate_envelope() {}\nmod helpers;\n")
            t.write("crates/sparq-x/src/helpers.rs",
                    "pub fn go() { crate::validate_envelope(); }\n")
            self.assertEqual(len(pf.check_guard_untested(ADDED, t.root)), 1)

    def test_a_cfg_not_test_mod_declaration_is_not_a_test_module(self) -> None:
        # `#[cfg(not(test))] mod shim;` compiles in ORDINARY builds. Kills:
        # accepting any cfg attribute on a `mod X;` rather than one that IMPLIES
        # test — the predicate must be shared with rust_cfg_test_spans.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/src/lib.rs",
                    "pub fn validate_envelope() {}\n#[cfg(not(test))]\nmod shim;\n")
            t.write("crates/sparq-x/src/shim.rs",
                    "pub fn go() { crate::validate_envelope(); }\n")
            self.assertEqual(len(pf.check_guard_untested(ADDED, t.root)), 1)

    def test_a_cfg_test_mod_with_a_BODY_is_not_treated_as_a_file(self) -> None:
        # `#[cfg(test)] mod tests { .. }` is the in-file form, already handled by
        # rust_cfg_test_spans. It must not ALSO resolve to a sibling `tests.rs`
        # and drag an unrelated production file in.
        self.assertEqual(
            pf.rust_cfg_test_mod_decls(pf.mask_rust("#[cfg(test)]\nmod tests { fn a() {} }\n")),
            [],
        )
        self.assertEqual(
            pf.rust_cfg_test_mod_decls(pf.mask_rust("#[cfg(test)]\nmod tests;\n")), ["tests"]
        )

    def test_mod_declarations_inside_comments_and_strings_do_not_resolve(self) -> None:
        # The scan runs over the MASKED text for the same reason the span scan
        # does — otherwise a doc example could admit an arbitrary file.
        src = ('/// #[cfg(test)]\n/// mod fake;\n'
               'const S: &str = "#[cfg(test)] mod alsofake;";\n')
        self.assertEqual(pf.rust_cfg_test_mod_decls(pf.mask_rust(src)), [])

    def test_module_file_resolution_matches_rustc_lookup(self) -> None:
        # Kills: dropping the lib/main/mod special case, which would look for
        # `src/lib/tests.rs` instead of `src/tests.rs`.
        self.assertEqual(
            pf.rust_module_files("crates/c/src/lib.rs", "tests"),
            ["crates/c/src/tests.rs", "crates/c/src/tests/mod.rs"],
        )
        self.assertEqual(
            pf.rust_module_files("crates/c/src/a/planner.rs", "tests"),
            ["crates/c/src/a/planner/tests.rs", "crates/c/src/a/planner/tests/mod.rs"],
        )
        self.assertEqual(
            pf.rust_module_files("crates/c/src/a/mod.rs", "tests"),
            ["crates/c/src/a/tests.rs", "crates/c/src/a/tests/mod.rs"],
        )


class PythonTestRegionsComeFromTheParser(unittest.TestCase):
    """REGRESSION (round 3) — the indentation heuristic truncated real self-tests.

    `python_test_regions` used to take the header line plus every following line
    that was blank or began with a space, tab or `)`. That is not a Python block,
    and it silently cut two self-tests short at the first column-0 line inside the
    body. Both shapes are in this tree today:

      * a `#` comment at column 0 inside a body (scripts/export-kb-dump.py) —
        this one DID produce a firing, `run_leak_check`, for a symbol its own
        `--self-test` names 4 lines past the cut;
      * column-0 content inside a triple-quoted fixture
        (scripts/check-spec-normative-status.py) — no symbol in that file fires
        either way. Found by differencing the heuristic against `ast` across
        every `scripts/*.py`, not by a false positive.

    Two holes in one heuristic is the design being wrong, so the heuristic is
    gone and `ast` decides the extent. These tests red on the heuristic.
    """

    GUARD = "def validate_lease(x):\n    return x\n"
    ADDED = {"scripts/thing.py": ["def validate_lease(x):"]}

    def test_a_column0_comment_does_not_end_a_python_test_region(self) -> None:
        # RED on the line heuristic: the region stopped at the `#` line and the
        # assert below it was never searched.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("scripts/thing.py",
                    self.GUARD
                    + "def self_test():\n"
                      "    ok = True\n"
                      "# a column-0 comment inside a body is legal Python\n"
                      "    assert validate_lease(1) == 1 and ok\n")
            self.assertEqual(pf.check_guard_untested(self.ADDED, t.root), [])

    def test_column0_text_in_a_triple_quoted_string_does_not_end_a_region(self) -> None:
        # RED on the line heuristic: a fixture whose content starts at column 0
        # ended the region on its second line.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("scripts/thing.py",
                    self.GUARD
                    + 'def self_test():\n'
                      '    fixture = """\n'
                      '== a heading at column 0 ==\n'
                      '"""\n'
                      '    assert validate_lease(fixture) is fixture\n')
            self.assertEqual(pf.check_guard_untested(self.ADDED, t.root), [])

    def test_a_test_class_body_is_a_region(self) -> None:
        # The `class *Test*` branch had no test of its own. Kills: dropping the
        # ClassDef arm.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("scripts/thing.py",
                    self.GUARD
                    + "class LeaseTests:\n"
                      "    def test_it(self):\n"
                      "        assert validate_lease(1) == 1\n")
            self.assertEqual(pf.check_guard_untested(self.ADDED, t.root), [])

    def test_a_nested_test_def_is_not_a_top_level_region(self) -> None:
        # Kills: walking ast.walk() instead of tree.body — a `def test_x` nested
        # inside an ordinary function is not a test the runner ever collects, and
        # accepting it would let any helper launder a call site into a test.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("scripts/thing.py",
                    self.GUARD
                    + "def helper():\n"
                      "    def test_inner():\n"
                      "        validate_lease(1)\n"
                      "    return test_inner\n")
            self.assertEqual(len(pf.check_guard_untested(self.ADDED, t.root)), 1)

    def test_a_script_that_does_not_parse_contributes_no_test_text(self) -> None:
        # FAIL-CLOSED. A file `ast` cannot parse is not evidence of a test.
        # Kills: falling back to the whole file (or to the old heuristic) on
        # SyntaxError, which would make an unparseable script satisfy every
        # guard it mentions anywhere.
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("scripts/thing.py",
                    self.GUARD
                    + "def self_test():\n"
                      "    assert validate_lease(1) == 1\n"
                      "def (\n")
            self.assertEqual(len(pf.check_guard_untested(self.ADDED, t.root)), 1)
            self.assertEqual(
                pf.python_test_regions((t.root / "scripts/thing.py").read_text()), ""
            )


class RustMaskingIsLiteralSafe(unittest.TestCase):
    """Brace matching must not be steered by braces inside strings or comments."""

    def test_a_brace_in_a_string_literal_does_not_open_a_block(self) -> None:
        # Kills: brace-matching the RAW text. `"{"` would open a block that never
        # closes, swallowing the rest of the file back into the test region.
        src = ('#[cfg(test)]\nmod tests {\n  const S: &str = "{";\n'
               '  #[test] fn t() {}\n}\npub fn validate_envelope() {}\n')
            # ^ the guard is defined AFTER the test mod on purpose
        masked = pf.mask_rust(src)
        spans = pf.rust_cfg_test_spans(masked)
        self.assertEqual(len(spans), 1)
        region = src[spans[0][0]:spans[0][1]]
        self.assertNotIn("validate_envelope", region)

    def test_a_brace_in_a_line_comment_does_not_open_a_block(self) -> None:
        src = ("#[cfg(test)]\nmod tests {\n  // opening { brace in a comment\n"
               "  #[test] fn t() {}\n}\npub fn validate_envelope() {}\n")
        spans = pf.rust_cfg_test_spans(pf.mask_rust(src))
        self.assertEqual(len(spans), 1)
        self.assertNotIn("validate_envelope", src[spans[0][0]:spans[0][1]])

    def test_mask_rust_preserves_offsets(self) -> None:
        # Every span is sliced out of the ORIGINAL text, so the mask must be the
        # same length or the regions come out shifted.
        src = 'fn f() { let s = "a{b}c"; /* } */ }\n'
        self.assertEqual(len(pf.mask_rust(src)), len(src))

    def test_an_unterminated_string_does_not_swallow_the_file_into_a_region(self) -> None:
        # FAIL-CLOSED on malformed input. An unterminated `"` blanks the rest of
        # the file, so the test mod's closing `}` disappears and the brace walk
        # reaches EOF with the block still open. Admitting that span would drag
        # every later definition into the "test text" and silence every guard
        # below it — a fail-open in the direction round-1 blocking 1 lived in.
        # Kills: appending the span regardless of the final depth.
        src = ('#[cfg(test)]\nmod tests { const S: &str = "oops;\n}\n'
               "pub fn validate_envelope() {}\n")
        self.assertEqual(pf.rust_cfg_test_spans(pf.mask_rust(src)), [])
        self.assertNotIn("validate_envelope",
                         pf.searchable_test_text("crates/c/src/lib.rs", src))

    def test_a_cfg_test_use_statement_opens_no_region(self) -> None:
        # `#[cfg(test)] use foo::bar;` has no body. Walking past the `;` looking
        # for a `{` would attach the NEXT item's body as a test region.
        src = ("#[cfg(test)]\nuse std::fmt;\n"
               "pub fn validate_envelope() {}\n")
        self.assertEqual(pf.rust_cfg_test_spans(pf.mask_rust(src)), [])


class TheDelegateRegistryIsPinned(unittest.TestCase):
    """`DELEGATES` is the mechanism this PR exists for — shifting the repo's OWN
    merge-gates left into the worker's worktree. It shipped with no test at all:
    all 7 mutants of it survived a green suite. These pin the registry ITSELF.

    Asserted against the imported `pf.DELEGATES` objects, not against the source
    text of preflight.py — a word-search over file text is exactly the mistake
    that produced the `guard-untested` corpus defect.
    """

    # (delegate name substring, the gate script it must run)
    REQUIRED = (
        ("G1", "scripts/gate-new-crate.py"),
        ("G2", "scripts/gate-api-skill.py"),
        ("G6", "scripts/check-config-documented.py"),
        ("no-perf-numbers", "scripts/check-no-perf-numbers.py"),
        ("readme-template", "scripts/check-readme-template.py"),
        ("privacy-claims", "scripts/check-privacy-claims.sh"),
    )

    def _by_script(self) -> dict[str, object]:
        return {d.script: d for d in pf.DELEGATES}

    def test_every_required_repo_gate_is_delegated(self) -> None:
        # Kills: `DELEGATES = []`, and dropping ANY single delegate (e.g. G1).
        have = self._by_script()
        missing = [f"{n} ({s})" for n, s in self.REQUIRED if s not in have]
        self.assertEqual(missing, [], f"preflight no longer runs: {missing}")

    def test_every_delegated_script_exists_in_the_repo(self) -> None:
        # Kills: a typo'd or renamed path, which previously degraded to a silent
        # `skipped` entry with a PASS exit code.
        absent = [d.script for d in pf.DELEGATES if not (REPO_ROOT / d.script).exists()]
        self.assertEqual(absent, [], f"delegated scripts that do not exist: {absent}")

    def test_each_delegate_name_matches_its_script(self) -> None:
        # Kills: rewiring a delegate's argv to a different (e.g. weaker) script
        # while keeping the reassuring name — the report would still say "ran G1".
        for name_part, script in self.REQUIRED:
            d = self._by_script()[script]
            self.assertIn(name_part, d.name)

    def test_the_enforcing_gates_are_run_with_enforce(self) -> None:
        # Kills: dropping `--enforce`. Both of these gates are ADVISORY without
        # it — they print findings and exit 0, so preflight would report PASS.
        for script in ("scripts/check-no-perf-numbers.py", "scripts/check-readme-template.py"):
            self.assertIn("--enforce", self._by_script()[script].argv,
                          f"{script} is delegated WITHOUT --enforce, so it cannot fail")

    def test_the_diff_scoped_gates_receive_the_changed_file_list(self) -> None:
        # Kills: clearing pass_changed_files, which would run G1/G2/G6 over the
        # whole tree and drown the author in pre-existing findings.
        for script in ("scripts/gate-new-crate.py", "scripts/gate-api-skill.py",
                       "scripts/check-config-documented.py"):
            self.assertTrue(self._by_script()[script].pass_changed_files,
                            f"{script} is not diff-scoped")

    def test_the_script_property_points_at_the_script_not_the_interpreter(self) -> None:
        for d in pf.DELEGATES:
            self.assertNotIn(d.script, ("python3", "bash", "sh"))
            self.assertTrue(d.script.startswith("scripts/"), d.script)


class _FakeGateTree:
    """A tree with stub gate scripts whose exit code we choose."""

    def __init__(self, td: str) -> None:
        self.root = Path(td)
        (self.root / "scripts").mkdir(parents=True, exist_ok=True)

    def gate(self, rel: str, rc: int) -> None:
        p = self.root / rel
        p.write_text(f"import sys\nprint('stub {rel} argv=' + ' '.join(sys.argv[1:]))\n"
                     f"sys.exit({rc})\n")

    def argv_seen(self, rel: str) -> Path:
        return self.root / (rel + ".argv")


def _delegate(name: str, rel: str, **kw):
    return pf.Delegate(name, ["python3", rel], **kw)


class RunDelegatesBehaviour(unittest.TestCase):
    """What `run_delegates` DOES — pinned end to end against real subprocesses."""

    def test_a_failing_delegate_becomes_a_finding(self) -> None:
        # THE headline behaviour. Kills: `if proc.returncode != 0:` -> `if False:`
        # (a FAILING gate reported as PASS), and `run_delegates` returning an
        # empty Result.
        with tempfile.TemporaryDirectory() as td:
            t = _FakeGateTree(td)
            t.gate("scripts/red.py", 1)
            with mock.patch.object(pf, "DELEGATES", [_delegate("RED gate", "scripts/red.py")]):
                res = pf.run_delegates(["a.rs"], t.root, None)
            self.assertEqual([f.check for f in res.findings], ["RED gate"])
            self.assertFalse(res.ok)

    def test_a_passing_delegate_produces_no_finding(self) -> None:
        # The other direction — without this the previous test also passes when
        # run_delegates flags everything unconditionally.
        with tempfile.TemporaryDirectory() as td:
            t = _FakeGateTree(td)
            t.gate("scripts/green.py", 0)
            with mock.patch.object(pf, "DELEGATES", [_delegate("GREEN gate", "scripts/green.py")]):
                res = pf.run_delegates(["a.rs"], t.root, None)
            self.assertEqual(res.findings, [])
            self.assertEqual(res.ran, ["GREEN gate"])

    def test_a_missing_gate_script_is_a_finding_not_a_skip(self) -> None:
        # REGRESSION — this FAILED OPEN. A missing script appended to `skipped`
        # and preflight still exited 0, so renaming any gate script silently
        # turned that gate off. A gate that did not run is not a gate that passed.
        with tempfile.TemporaryDirectory() as td:
            t = _FakeGateTree(td)
            with mock.patch.object(pf, "DELEGATES", [_delegate("GONE gate", "scripts/gone.py")]):
                res = pf.run_delegates(["a.rs"], t.root, None)
            self.assertEqual([f.check for f in res.findings], ["GONE gate"])
            self.assertNotIn("GONE gate", " ".join(res.skipped))

    def test_the_changed_files_path_is_passed_to_a_diff_scoped_delegate(self) -> None:
        # Kills: never appending `--changed-files`, which un-scopes every gate.
        with tempfile.TemporaryDirectory() as td:
            t = _FakeGateTree(td)
            t.gate("scripts/echo.py", 0)
            cf = t.root / "changed.txt"
            cf.write_text("a.rs\n")
            d = _delegate("ECHO gate", "scripts/echo.py", pass_changed_files=True)
            with mock.patch.object(pf, "DELEGATES", [d]):
                with mock.patch.object(pf.subprocess, "run",
                                       wraps=pf.subprocess.run) as spy:
                    pf.run_delegates(["a.rs"], t.root, cf)
            argv = spy.call_args_list[-1].args[0]
            self.assertIn("--changed-files", argv)
            self.assertIn(str(cf), argv)

    def test_a_path_filtered_delegate_is_skipped_when_nothing_matches(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            t = _FakeGateTree(td)
            t.gate("scripts/md.py", 1)   # would FAIL if it ran
            d = _delegate("MD gate", "scripts/md.py", path_filter=re.compile(r"\.md$"))
            with mock.patch.object(pf, "DELEGATES", [d]):
                res = pf.run_delegates(["a.rs"], t.root, None)
            self.assertEqual(res.findings, [])
            self.assertEqual(res.ran, [])

    def test_a_path_filtered_delegate_runs_when_a_path_matches(self) -> None:
        # The quantifier check on the previous test: the filter must not skip
        # EVERYTHING.
        with tempfile.TemporaryDirectory() as td:
            t = _FakeGateTree(td)
            t.gate("scripts/md.py", 1)
            d = _delegate("MD gate", "scripts/md.py", path_filter=re.compile(r"\.md$"))
            with mock.patch.object(pf, "DELEGATES", [d]):
                res = pf.run_delegates(["README.md"], t.root, None)
            self.assertEqual([f.check for f in res.findings], ["MD gate"])

    def test_main_propagates_a_delegate_finding_to_the_exit_code(self) -> None:
        # THE call site. Kills: `main()` discarding `sub.findings` — every gate
        # could then fail and preflight would still print PASS and exit 0.
        with tempfile.TemporaryDirectory() as td:
            t = _FakeGateTree(td)
            t.gate("scripts/red.py", 1)
            cf = t.root / "changed.txt"
            cf.write_text("a.rs\n")
            with mock.patch.object(pf, "DELEGATES", [_delegate("RED gate", "scripts/red.py")]):
                rc = pf.main(["--root", str(t.root), "--changed-files", str(cf),
                              "--only", "delegates", "--quiet"])
            self.assertEqual(rc, 1)

    def test_main_reports_PASS_only_when_no_delegate_failed(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            t = _FakeGateTree(td)
            t.gate("scripts/green.py", 0)
            cf = t.root / "changed.txt"
            cf.write_text("a.rs\n")
            with mock.patch.object(pf, "DELEGATES", [_delegate("GREEN", "scripts/green.py")]):
                rc = pf.main(["--root", str(t.root), "--changed-files", str(cf),
                              "--only", "delegates", "--quiet"])
            self.assertEqual(rc, 0)


class ExitCodeContract(unittest.TestCase):
    """A finding must make the process exit non-zero — no exit-zero swallowing."""

    def test_main_exits_nonzero_on_a_finding(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            cf = Path(td) / "changed.txt"
            cf.write_text("crates/sparq-x/src/lib.rs\n")
            al = Path(td) / "added.diff"
            al.write_text("--- a/crates/sparq-x/src/lib.rs\n"
                          "+++ b/crates/sparq-x/src/lib.rs\n"
                          "@@ -0,0 +1 @@\n"
                          "+pub fn validate_envelope() {}\n")
            rc = pf.main(["--root", str(t.root), "--changed-files", str(cf),
                          "--added-lines", str(al), "--only", "guard-untested", "--quiet"])
            self.assertEqual(rc, 1)

    def test_main_exits_zero_when_clean(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            t = _Tree(td)
            t.write("crates/sparq-x/tests/t.rs",
                    "#[test]\nfn t() { sparq_x::validate_envelope(); }\n")
            cf = Path(td) / "changed.txt"
            cf.write_text("crates/sparq-x/src/lib.rs\n")
            al = Path(td) / "added.diff"
            al.write_text("--- a/crates/sparq-x/src/lib.rs\n"
                          "+++ b/crates/sparq-x/src/lib.rs\n"
                          "@@ -0,0 +1 @@\n"
                          "+pub fn validate_envelope() {}\n")
            rc = pf.main(["--root", str(t.root), "--changed-files", str(cf),
                          "--added-lines", str(al), "--only", "guard-untested", "--quiet"])
            self.assertEqual(rc, 0)


class NonMechanicalObligationsAreStated(unittest.TestCase):
    """The script must never imply it has checked what it cannot check."""

    def test_the_mutation_obligation_is_printed(self) -> None:
        # Kills: deleting the NON_MECHANICAL block, which would leave a PASS reading
        # as "this diff is reviewed" rather than "the mechanical gates are green".
        self.assertIn("MUTATE YOUR HEADLINE GUARD", pf.NON_MECHANICAL)

    def test_the_claim_vs_code_obligation_is_printed(self) -> None:
        self.assertIn("READ YOUR OWN PROSE AGAINST YOUR OWN DIFF", pf.NON_MECHANICAL)


WORKER_BRIEFS = (
    "sparq-rust-impl.md",
    "sparq-rust-feature.md",
    "sparq-ci-infra.md",
    "sparq-docs.md",
    "sparq-site.md",
    "sparq-perf-engineer.md",
)


class WorkerBriefsCarryThePresubmitBlock(unittest.TestCase):
    """One brief silently losing a clause is a known, repeated failure here.

    A worker reads only its OWN brief, so an obligation that lives in five of six
    is absent for the sixth. These tests red the moment a brief drops the block or
    the copies drift apart.
    """

    def _briefs(self) -> dict[str, str]:
        return {
            n: (REPO_ROOT / ".claude" / "agents" / n).read_text()
            for n in WORKER_BRIEFS
        }

    def test_every_worker_brief_names_the_preflight_runner(self) -> None:
        missing = [n for n, t in self._briefs().items() if "scripts/preflight.py" not in t]
        self.assertEqual(missing, [], f"worker briefs missing the preflight step: {missing}")

    def test_every_worker_brief_carries_the_mutation_obligation(self) -> None:
        missing = [n for n, t in self._briefs().items() if "MUTATE YOUR HEADLINE GUARD" not in t]
        self.assertEqual(missing, [], f"briefs missing the mutation obligation: {missing}")

    def test_every_worker_brief_carries_the_claim_vs_code_obligation(self) -> None:
        missing = [
            n for n, t in self._briefs().items()
            if "READ YOUR OWN PROSE AGAINST YOUR OWN DIFF" not in t
        ]
        self.assertEqual(missing, [], f"briefs missing the claim-vs-code obligation: {missing}")

    def test_the_block_is_byte_identical_across_briefs(self) -> None:
        # Divergent copies are how a "shared contract" quietly stops being shared.
        marker = "## Before you open the PR (HARD — identical in every worker brief)"
        blocks = {}
        for n, t in self._briefs().items():
            self.assertIn(marker, t, f"{n} lost the block header")
            body = t.split(marker, 1)[1]
            # the block ends at the next top-level heading, or EOF
            end = body.find("\n## ")
            blocks[n] = body[:end] if end != -1 else body.rstrip() + "\n"
        distinct = set(b.rstrip() for b in blocks.values())
        self.assertEqual(
            len(distinct), 1,
            "the pre-submit block has diverged across worker briefs:\n"
            + "\n".join(f"  {n}: {len(b)} chars" for n, b in blocks.items()),
        )


class TheYamlSeamIsGating(unittest.TestCase):
    """The wiring, not just the logic.

    Measured on this repo: in an 18-mutant run every UNCAUGHT mutant lived at the
    workflow `if:`/step/call-site, not in the Python. A checker wired into an
    advisory job, guarded by an `if:`, or carrying `continue-on-error` is a checker
    that does not check. These tests red on each of those.
    """

    WORKFLOW = ".github/workflows/docs-quality.yml"
    STEP_RUNS = (
        "python3 scripts/preflight.py --self-test",
        "python3 scripts/tests/test_preflight.py",
    )

    def _job_hosting(self, run_cmd: str):
        import yaml

        doc = yaml.safe_load((REPO_ROOT / self.WORKFLOW).read_text())
        for jid, job in doc["jobs"].items():
            for step in job.get("steps", []):
                if run_cmd in str(step.get("run", "")):
                    return jid, job, step
        return None, None, None

    def test_both_preflight_legs_are_wired_into_the_workflow(self) -> None:
        for cmd in self.STEP_RUNS:
            jid, _job, _step = self._job_hosting(cmd)
            self.assertIsNotNone(jid, f"no step in {self.WORKFLOW} runs: {cmd}")

    def test_the_hosting_job_name_carries_no_advisory_token(self) -> None:
        # ci-summary EXCLUDES any check-run whose name matches
        # \b(advisory|informational|non-blocking)\b. Renaming the job to include one
        # of those tokens silently un-gates every leg in it.
        import re as _re

        for cmd in self.STEP_RUNS:
            jid, job, _step = self._job_hosting(cmd)
            assert job is not None
            name = str(job.get("name") or jid)
            self.assertIsNone(
                _re.search(r"\b(advisory|informational|non-blocking)\b", name, _re.I),
                f"{cmd} is hosted by job {name!r}, which ci-summary would EXCLUDE",
            )

    def test_neither_leg_can_swallow_its_own_failure(self) -> None:
        # continue-on-error at EITHER the job or the step level turns a red leg
        # green. This is the exact shape of the exit-zero swallowing defect the
        # verdict corpus reports 11 times.
        for cmd in self.STEP_RUNS:
            jid, job, step = self._job_hosting(cmd)
            assert job is not None and step is not None
            self.assertNotEqual(job.get("continue-on-error"), True,
                                f"job {jid} hosting {cmd} is continue-on-error")
            self.assertNotEqual(step.get("continue-on-error"), True,
                                f"the step running {cmd} is continue-on-error")

    def test_neither_leg_is_conditionally_skipped(self) -> None:
        # An `if:` on the step is the cheapest way to make a gate vacuous.
        for cmd in self.STEP_RUNS:
            _jid, _job, step = self._job_hosting(cmd)
            assert step is not None
            self.assertIsNone(step.get("if"),
                              f"the step running {cmd} is guarded by an if:")

    def test_neither_leg_is_invoked_with_its_exit_code_discarded(self) -> None:
        # The commonest swallow shape, and the one a SUBSTRING match cannot see:
        # `python3 scripts/preflight.py --self-test || true` still "contains" the
        # command, so every other test in this class stays green while the leg can
        # no longer fail. Same for a trailing `; true`, `| tee`, or `set +e`.
        # The repo's own precedent is test_banned_terminology.py's anchored
        # bare-call regex; this copies that shape.
        # Kills: appending `|| true` to either step's run.
        import re as _re

        for cmd in self.STEP_RUNS:
            _jid, _job, step = self._job_hosting(cmd)
            assert step is not None
            run = str(step.get("run", ""))
            bare = _re.compile(r"^[ \t]*" + _re.escape(cmd) + r"[ \t]*$", _re.M)
            self.assertRegex(
                run, bare,
                f"{cmd} is not invoked as a bare command — its exit code can be "
                f"discarded by whatever follows it on the line:\n{run!r}",
            )


class SelfTestIsWired(unittest.TestCase):
    def test_self_test_passes(self) -> None:
        proc = subprocess.run(
            [sys.executable, str(REPO_ROOT / "scripts" / "preflight.py"), "--self-test"],
            capture_output=True, text=True,
        )
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
