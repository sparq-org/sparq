#!/usr/bin/env python3
# [OPUS-5] 🤖 SPARQ agent — guard suite for scripts/selftest_write_guard.py (sparq-org/sparq#5098).
#
# The defect this pins: `scripts/retriage-needs-user.py` (PR #4184) self-tested its "never writes"
# early-out by CALLING the write function with writes enabled and the target defaulted to
# `sparq-org/sparq`. Mutating the branch under test — which that PR advertised as its verification
# method — turned `--selftest` into a production mutation. Four such comments are live on issues #7
# and #8 (both closed dependabot PRs), each rendering the fixture reason string "r".
#
# THE FOUR FAMILIES HERE
#
#   1. KNOWN POSITIVES — the detector is run against the VERBATIM #5098 shape and against each
#      bypass of it (positional literal, partial seam neutralisation, live slug handed to a
#      cross-module callee). An instrument that has only ever been run on a clean tree has not been
#      shown to detect anything; these are the "validate against a known answer" rows.
#   2. KNOWN NEGATIVES — the shapes that MUST stay green: `scripts/batch-merge.py`'s correct
#      neutralisation pattern, `dry_run=True`, a fixture-owner slug, a target parameter with no
#      default. A guard that cannot be satisfied gets deleted.
#   3. THE REAL TREE — `scripts/**/*.py` has no violation, AND the scan is pointed at something
#      (non-zero self-tests / write functions / seams), so deleting the last real subject cannot
#      silently turn the tree-scan vacuous.
#   4. THE YAML SEAM — parsed STRUCTURALLY, never by substring. Measured in this project: every
#      surviving mutant lived in a workflow `if:` / step / call site, and a wiring pin that greps
#      text does NOT catch a job-level `if: false` (#5080). So: the `validate` job carries no `if:`,
#      no run step carries an `if:`, the run block INVOKES this suite, and both `paths:` filters
#      carry the DIRECTORY GLOB (an enumeration cannot cover a script that does not exist yet —
#      which is precisely how `retriage-needs-user.py` would have arrived unscanned).
#
# Run:  python3 scripts/tests/test_selftest_never_writes.py

from __future__ import annotations

import importlib.util
import sys
import textwrap
import unittest
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPTS = REPO_ROOT / "scripts"
GUARD = SCRIPTS / "selftest_write_guard.py"
ROUTING_WF = REPO_ROOT / ".github" / "workflows" / "routing-self-tests.yml"
SCRIPTS_GLOB = "scripts/**/*.py"


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader, path
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


guard = _load("selftest_write_guard", GUARD)


def _yaml(path: Path) -> dict:
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def _on_block(wf: dict) -> dict:
    """Bare `on` is the YAML boolean True, so PyYAML keys it under `True`."""
    return wf.get("on", wf.get(True, {})) or {}


def rules(source: str) -> list[str]:
    return sorted(v.rule for v in guard.check_source(textwrap.dedent(source), "<fixture>"))


# The #5098 defect, reduced to its load-bearing lines and otherwise verbatim.
THE_DEFECT = '''
    import subprocess
    REPO = "sparq-org/sparq"

    def _run(args):
        return subprocess.run(args, capture_output=True, text=True, check=True)

    def apply_one(issue, action, remove, add, reason, repo=REPO, dry_run=True, log=print):
        if action in ("keep", "skip"):
            return False
        if dry_run:
            return False
        _run(["gh", "issue", "edit", str(issue["n"]), "--repo", repo])
        _run(["gh", "issue", "comment", str(issue["n"]), "--repo", repo, "--body", reason])
        return True

    def _selftest():
        assert apply_one({"n": 7}, "keep", ["needs:user"], [], "r",
                         dry_run=False, log=lambda *_: None) is False
'''

# What `scripts/batch-merge.py` does, and what the fix for the above looks like.
THE_SAFE_PATTERN = '''
    import subprocess

    def run_gh(argv, capture=True):
        return subprocess.run(argv, capture_output=True, text=True).stdout

    def run_git(argv, check=True):
        return subprocess.run(argv).returncode

    def execute(actions, state, dry_run=True):
        if dry_run:
            return
        run_git(["merge", "x"])
        run_gh(["pr", "create"])

    def self_test():
        calls = []
        global run_git, run_gh
        real_git, real_gh = run_git, run_gh
        run_git, run_gh = lambda a, check=True: calls.append(a) or 0, lambda a, capture=True: ""
        try:
            execute([], {}, dry_run=False)
        finally:
            run_git, run_gh = real_git, real_gh
'''


class TestKnownPositives(unittest.TestCase):
    """Each row is a shape the guard MUST refuse. These are the rows that go red on the defect."""

    def test_the_5098_defect_trips_both_rules(self):
        self.assertEqual(rules(THE_DEFECT),
                         ["live-default-target", "selftest-writes-live"])

    def test_selftest_calling_a_write_function_with_dry_run_false_is_refused(self):
        """THE acceptance row for #5098: a `dry_run=False` self-test call reds this test."""
        found = guard.check_source(textwrap.dedent(THE_DEFECT), "retriage-needs-user.py")
        writes = [v for v in found if v.rule == "selftest-writes-live"]
        self.assertEqual(len(writes), 1, found)
        self.assertIn("dry_run", writes[0].detail)
        self.assertIn("_run", writes[0].detail)

    def test_positional_write_literal_is_not_a_bypass(self):
        """`apply_one(i, a, r, d, x, repo, False)` — the cheapest keyword-guard bypass."""
        self.assertIn("selftest-writes-live", rules('''
            import subprocess
            def _run(a):
                return subprocess.run(a)
            def apply_one(issue, repo, dry_run=True):
                if not dry_run:
                    _run(["gh", "issue", "edit"])
            def _selftest():
                apply_one({"n": 7}, "acme/widget", False)
        '''))

    def test_partial_neutralisation_is_refused(self):
        """Rebinding ONE of two reachable seams still leaves a live write path."""
        self.assertIn("selftest-writes-live", rules('''
            import subprocess
            def run_gh(a):
                return subprocess.run(a)
            def run_git(a):
                return subprocess.run(a)
            def execute(dry_run=True):
                if not dry_run:
                    run_git(["merge"])
                    run_gh(["pr", "create"])
            def self_test():
                global run_git
                run_git = lambda a: 0
                execute(dry_run=False)
        '''))

    def test_a_bare_global_declaration_is_not_neutralisation(self):
        """`global _run` with NO assignment leaves the REAL seam bound.

        Declaring the intent to swap a seam is not swapping it. Without this row the guard's
        `declared & assigned` intersection can be weakened to `declared` and every other test
        still passes — measured: that mutant survived the first full run of this suite.
        """
        self.assertIn("selftest-writes-live", rules('''
            import subprocess
            def _run(a):
                return subprocess.run(a)
            def apply_one(repo, dry_run=True):
                if not dry_run:
                    _run(["gh"])
            def _selftest():
                global _run
                apply_one("acme/widget", dry_run=False)
        '''))

    def test_transitive_seam_is_reached_through_a_helper(self):
        """The seam two hops from the callee still counts — `apply_one -> _post -> subprocess`."""
        self.assertIn("selftest-writes-live", rules('''
            import subprocess
            def _post(a):
                return subprocess.run(a)
            def _edit(a):
                return _post(a)
            def apply_one(repo, dry_run=True):
                if not dry_run:
                    _edit(["gh"])
            def _selftest():
                apply_one("acme/widget", dry_run=False)
        '''))

    def test_live_slug_handed_to_a_cross_module_callee_is_refused(self):
        """Callee defined elsewhere: the guard cannot see its seams, so a LIVE target is
        the test."""
        self.assertEqual(rules('''
            from writer import apply_one
            def _selftest():
                apply_one(7, repo="sparq-org/sparq", dry_run=False)
        '''), ["selftest-writes-live"])

    def test_live_default_target_is_refused_even_with_no_selftest(self):
        """Rule 1 stands alone: a write function must never inherit production by omission."""
        self.assertEqual(rules('''
            REPO = "jeswr/agent-account-registry"
            def apply_one(issue, repo=REPO, dry_run=True):
                return None
        '''), ["live-default-target"])

    def test_inline_live_default_is_refused(self):
        self.assertEqual(rules('''
            def apply_one(issue, repo="sparq-org/sparq", dry_run=True):
                return None
        '''), ["live-default-target"])


class TestKnownNegatives(unittest.TestCase):
    """Shapes that MUST stay green — a guard nobody can satisfy is a guard that gets deleted."""

    def test_the_batch_merge_neutralisation_pattern_is_accepted(self):
        self.assertEqual(rules(THE_SAFE_PATTERN), [])

    def test_the_5098_defect_repaired_is_accepted(self):
        """The fix: drop the live default, inject a recorder, keep the write-enabling call."""
        self.assertEqual(rules('''
            import subprocess
            REPO = "sparq-org/sparq"

            def _run(args):
                return subprocess.run(args, check=True)

            def apply_one(issue, action, repo, dry_run=True, log=print):
                if action in ("keep", "skip"):
                    return False
                if dry_run:
                    return False
                _run(["gh", "issue", "edit", str(issue["n"]), "--repo", repo])
                return True

            def _selftest():
                global _run
                real = _run
                _run = lambda args: None
                try:
                    apply_one({"n": 7}, "keep", "fixture/repo", dry_run=False)
                finally:
                    _run = real
        '''), [])

    def test_dry_run_true_in_a_selftest_is_fine(self):
        self.assertEqual(rules(THE_DEFECT.replace("dry_run=False", "dry_run=True")
                               .replace("repo=REPO", "repo")), [])

    def test_a_fixture_owner_default_is_not_a_live_repo(self):
        self.assertEqual(rules('''
            def apply_one(issue, repo="fixture/repo", dry_run=True):
                return None
        '''), [])

    def test_a_target_parameter_with_no_default_is_fine(self):
        self.assertEqual(rules('''
            def apply_one(issue, repo, dry_run=True):
                return None
        '''), [])

    def test_a_non_write_function_may_default_to_the_live_repo(self):
        """Read paths legitimately default to production — only WRITE functions are constrained."""
        self.assertEqual(rules('''
            REPO = "sparq-org/sparq"
            def fetch_needs_user(repo=REPO):
                return []
        '''), [])

    def test_a_selftest_with_no_reachable_seam_is_fine(self):
        self.assertEqual(rules('''
            def plan(state, dry_run=True):
                return []
            def _selftest():
                plan({}, dry_run=False)
        '''), [])


class TestTheRealTree(unittest.TestCase):
    """Family 3: the tree is clean AND the scan is pointed at something."""

    @classmethod
    def setUpClass(cls):
        cls.violations = guard.check_tree(SCRIPTS)

    def test_no_script_lets_a_selftest_write_to_a_live_repo(self):
        self.assertEqual([str(v) for v in self.violations], [])

    def test_the_scan_is_not_vacuous(self):
        """A clean report is only meaningful if the scan actually reached self-tests, write
        functions and subprocess seams. Without this row, deleting every subject would read as a
        pass."""
        import ast
        selftests = writers = seams = 0
        files = 0
        for path in sorted(SCRIPTS.rglob("*.py")):
            try:
                tree = ast.parse(path.read_text(encoding="utf-8", errors="replace"))
            except SyntaxError:
                continue
            files += 1
            seams += len(guard.direct_seams(tree))
            for fn in ast.walk(tree):
                if not isinstance(fn, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    continue
                if guard.SELFTEST_NAME.match(fn.name):
                    selftests += 1
                args = fn.args
                if {a.arg for a in args.posonlyargs + args.args + args.kwonlyargs} \
                        & guard.DRY_RUN_PARAMS:
                    writers += 1
        self.assertGreater(files, 100, "the scan is not reaching scripts/")
        self.assertGreater(selftests, 10, "no self-tests scanned — rule 2 has no population")
        self.assertGreater(writers, 5, "no write functions scanned — rule 1 has no population")
        self.assertGreater(seams, 10, "no subprocess seams found — the seam analysis is blind")


class TestWorkflowWiring(unittest.TestCase):
    """Family 4: the YAML seam. Substring assertions do NOT catch `if: false` — parse it."""

    @classmethod
    def setUpClass(cls):
        cls.wf = _yaml(ROUTING_WF)
        cls.job = cls.wf["jobs"]["validate"]

    def _run_block(self) -> str:
        return " ".join(" ".join(s["run"] for s in self.job["steps"] if "run" in s).split())

    def test_the_validate_job_has_no_if_guard(self):
        """MUTANT: `if: false` on the job => RED (a text grep survives this — #5080)."""
        self.assertNotIn("if", self.job)

    def test_no_run_step_carries_an_if_guard(self):
        """MUTANT: `if: false` on the step that invokes this suite => RED."""
        for step in self.job["steps"]:
            if "run" in step:
                self.assertNotIn("if", step,
                                 f"run step {step.get('name')!r} is conditionally skipped")

    def test_routing_self_tests_invokes_this_suite(self):
        """MUTANT: delete the call site => RED. A suite nobody runs cannot go red."""
        self.assertIn("python3 scripts/tests/test_selftest_never_writes.py", self._run_block())

    def test_both_paths_filters_carry_the_scripts_glob(self):
        """MUTANT: drop the glob => RED.

        A DIRECTORY GLOB, not an enumeration: the guard scans `scripts/**/*.py`, so a script that
        does not exist yet must still trigger the lane. `retriage-needs-user.py` was exactly such a
        file — an enumeration written today could not have named it.
        """
        on = _on_block(self.wf)
        for event in ("pull_request", "push"):
            paths = set(on[event].get("paths") or [])
            self.assertIn(SCRIPTS_GLOB, paths,
                          f"{event}.paths does not watch {SCRIPTS_GLOB} — a new script could add "
                          f"a writing self-test without this lane ever running")

    def test_the_workflow_watches_itself(self):
        on = _on_block(self.wf)
        for event in ("pull_request", "push"):
            self.assertIn(".github/workflows/routing-self-tests.yml",
                          set(on[event].get("paths") or []))


if __name__ == "__main__":
    unittest.main(verbosity=2)
