#!/usr/bin/env python3
# [OPUS-5] sparq-org/sparq#3760 — cross-file INSPECTION pins for the arm path.
#
# THE OUTAGE. The re-arm sweeper could not enable auto-merge at all:
#   run 30033315483 (2026-07-23T18:21Z) failed arming PR #3454 (OPEN, non-draft, CLEAN,
#   review:pass) with "GraphQL: Resource not accessible by integration
#   (enablePullRequestAutoMerge)", and had been doing so every ten minutes.
# THE CAUSE. rearm-sweeper.yml declared `permissions: contents: read`. Enabling auto-merge
# is a repository WRITE operation. auto-arm.yml armed two PRs 90 minutes earlier
# (run 30027010495, 16:52Z) on the same repository with the same plain GITHUB_TOKEN — its
# only difference was `contents: write`. `allow_auto_merge` was already true and the `main`
# ruleset carried no integration restriction, so nothing maintainer-only was involved.
#
# A `permissions:` block cannot be unit-tested by execution, so this suite pins the SHAPE
# of everything the fix depends on:
#   * PERMISSION PIN — both arm workflows must declare `contents: write` AND
#     `pull-requests: write`. A silent downgrade to `read` reproduces #3760 exactly, and
#     it is invisible in a diff review of a python-only change.
#   * PROBE WIRED, AND FIRST — each workflow must run `--probe-arm-capability` as its own
#     step BEFORE the arming step, so a token that cannot arm reds the job once instead of
#     failing per-PR forever.
#   * SAME TOKEN — the probe step's GH_TOKEN expression must be byte-identical to the arm
#     step's. A probe that attests a different token than the sweep uses is worse than no
#     probe: it would report can-arm while the sweep is denied.
#   * REMEDIATION CONTENT — the single ::error must name every flippable thing
#     (`contents: write`, the repository "Allow auto-merge" setting, and the
#     ORCHESTRATOR_APP_ID / ORCHESTRATOR_APP_PRIVATE_KEY secrets). An ::error that says
#     only "permission denied" is what made this cost days.
#   * NO-DRIFT — the two scripts must stay self-contained (each workflow sparse-checks out
#     only its own file, so they CANNOT import a shared helper) while keeping the same
#     denial-marker set. This pins the duplication instead of letting it rot.
#   * EXIT SEMANTICS — neither script may lose its non-zero exit on an arm failure.
#   * STICKY PRECEDENCE (#3766) — collecting failures is only half the job. Both scripts
#     must compute the exit from the FINAL accumulated state with the precedence
#     `collected-failure > transient-exhaustion > clean`, so a LATER candidate's exhausted
#     transient can never downgrade an EARLIER candidate's real arm failure to the lenient
#     ::warning + exit 0. Pinned in both (deliberately duplicated) files.
#
# Needs PyYAML (already a docs-quality dependency); everything else is stdlib. Run:
#   python3 scripts/tests/test_arm_capability_wiring.py

from __future__ import annotations

import ast
import copy
import importlib.util
import json
import re
import shutil
import subprocess
import socket
import sys
import tempfile
import textwrap
import unittest
from unittest.mock import patch
from pathlib import Path
from types import ModuleType
from typing import NamedTuple

import yaml

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
REARM_YML = WORKFLOWS / "rearm-sweeper.yml"
AUTO_ARM_YML = WORKFLOWS / "auto-arm.yml"
REARM_PY = REPO_ROOT / "scripts" / "rearm-sweeper.py"
AUTO_ARM_PY = REPO_ROOT / "scripts" / "auto-arm.py"

PROBE_FLAG = "--probe-arm-capability"
# The exact string GitHub returned in run 30033315483 — the regression anchor.
LIVE_DENIAL = (
    "GraphQL: Resource not accessible by integration (enablePullRequestAutoMerge)"
)
# Every remediation lever a human can actually pull, by exact name.
REQUIRED_REMEDIATION_TOKENS = (
    "contents: write",
    "pull-requests: write",
    "Allow auto-merge",
    "autoMergeAllowed",
    "ORCHESTRATOR_APP_ID",
    "ORCHESTRATOR_APP_PRIVATE_KEY",
)

ARM_WORKFLOWS = {
    "rearm-sweeper.yml": (REARM_YML, "scripts/rearm-sweeper.py"),
    "auto-arm.yml": (AUTO_ARM_YML, "scripts/auto-arm.py"),
}


def load(path: Path) -> dict:
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def load_module(path: Path, name: str) -> ModuleType:
    """Import a hyphenated script by path (the scripts are not importable by name)."""
    # Both scripts import the sibling gh_retry helper (#3759), which is only importable
    # with scripts/ on the path — the workflows get that for free by running from there.
    scripts_dir = str(REPO_ROOT / "scripts")
    if scripts_dir not in sys.path:
        sys.path.insert(0, scripts_dir)
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None, path
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def steps_of(document: dict) -> list[dict]:
    steps: list[dict] = []
    for job in (document.get("jobs") or {}).values():
        steps.extend(job.get("steps") or [])
    return steps


def run_of(step: dict) -> str:
    return str(step.get("run") or "")


class TestPermissionPin(unittest.TestCase):
    """#3760: `contents: read` on an arming job IS the bug. Pin write on both."""

    def test_both_arm_workflows_declare_contents_and_pull_requests_write(self) -> None:
        for name, (path, _script) in ARM_WORKFLOWS.items():
            document = load(path)
            permissions = document.get("permissions")
            self.assertIsInstance(
                permissions,
                dict,
                f"{name} must declare an explicit permissions block",
            )
            self.assertEqual(
                permissions.get("contents"),
                "write",
                f"{name}: enablePullRequestAutoMerge is a repository WRITE operation; "
                f"`contents: read` reproduces #3760 ({LIVE_DENIAL})",
            )
            self.assertEqual(
                permissions.get("pull-requests"),
                "write",
                f"{name} must keep `pull-requests: write`",
            )

    def test_arm_workflows_do_not_narrow_permissions_at_the_job_level(self) -> None:
        # A job-level `permissions:` REPLACES the workflow-level block, so a job-level
        # narrowing would silently reintroduce the outage.
        for name, (path, _script) in ARM_WORKFLOWS.items():
            for job_id, job in (load(path).get("jobs") or {}).items():
                if "permissions" not in job:
                    continue
                permissions = job["permissions"]
                self.assertIsInstance(permissions, dict, f"{name}:{job_id}")
                self.assertEqual(
                    permissions.get("contents"), "write", f"{name}:{job_id}"
                )
                self.assertEqual(
                    permissions.get("pull-requests"), "write", f"{name}:{job_id}"
                )


class TestProbeWiring(unittest.TestCase):
    """The probe must exist, run first, and attest the token the arm step uses."""

    def test_probe_step_exists_and_precedes_the_arm_step(self) -> None:
        for name, (path, script) in ARM_WORKFLOWS.items():
            steps = steps_of(load(path))
            probe_indexes = [
                index
                for index, step in enumerate(steps)
                if PROBE_FLAG in run_of(step) and script in run_of(step)
            ]
            arm_indexes = [
                index
                for index, step in enumerate(steps)
                if script in run_of(step)
                and PROBE_FLAG not in run_of(step)
                and "--self-test" not in run_of(step)
            ]
            self.assertEqual(
                len(probe_indexes), 1, f"{name} must run {PROBE_FLAG} exactly once"
            )
            self.assertTrue(arm_indexes, f"{name} must still have an arming step")
            self.assertLess(
                probe_indexes[0],
                min(arm_indexes),
                f"{name}: the capability probe must run BEFORE any arm",
            )

    def test_probe_and_arm_steps_use_the_identical_token_expression(self) -> None:
        for name, (path, script) in ARM_WORKFLOWS.items():
            steps = steps_of(load(path))
            probe = next(
                step
                for step in steps
                if PROBE_FLAG in run_of(step) and script in run_of(step)
            )
            arm = next(
                step
                for step in steps
                if script in run_of(step)
                and PROBE_FLAG not in run_of(step)
                and "--self-test" not in run_of(step)
            )
            probe_token = (probe.get("env") or {}).get("GH_TOKEN")
            arm_token = (arm.get("env") or {}).get("GH_TOKEN")
            self.assertTrue(probe_token, f"{name}: probe step needs GH_TOKEN")
            self.assertEqual(
                probe_token,
                arm_token,
                f"{name}: a probe on a DIFFERENT token than the arm step would attest "
                "capability the sweep does not have",
            )

    def test_self_test_step_survives(self) -> None:
        for name, (path, script) in ARM_WORKFLOWS.items():
            steps = steps_of(load(path))
            self.assertTrue(
                any(
                    "--self-test" in run_of(step) and script in run_of(step)
                    for step in steps
                ),
                f"{name} must keep running the policy self-test before arming",
            )


class TestScriptContract(unittest.TestCase):
    """Pin the behaviour the workflows depend on, in both (deliberately duplicated) files."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.rearm = load_module(REARM_PY, "rearm_sweeper_under_test")
        cls.auto = load_module(AUTO_ARM_PY, "auto_arm_under_test")

    def test_both_expose_the_probe_and_denial_classifier(self) -> None:
        for module in (self.rearm, self.auto):
            self.assertTrue(callable(module.is_arm_denial))
            self.assertTrue(callable(module.probe_arm_capability_exit))
            self.assertEqual(module.CAN_ARM, "can-arm")
            self.assertEqual(module.CANNOT_ARM, "cannot-arm")
            self.assertEqual(module.INCONCLUSIVE, "inconclusive")

    def test_the_live_denial_text_classifies_as_a_capability_denial(self) -> None:
        for module in (self.rearm, self.auto):
            self.assertTrue(
                module.is_arm_denial(f"gh pr merge 3454 failed: {LIVE_DENIAL}"),
                "the exact run-30033315483 error must be recognised",
            )

    def test_per_pr_conditions_are_not_misclassified_as_capability_denials(self) -> None:
        # A race/CAS/transient must stay per-PR, or one flaky PR would stop the sweep.
        for module in (self.rearm, self.auto):
            for benign in (
                "Head branch was modified. Review and try again",
                "HTTP 502: 502 Bad Gateway",
                "Pull request is in unstable status",
                "simulated expectedHeadOid CAS rejection",
            ):
                self.assertFalse(module.is_arm_denial(benign), (module.PROGRAM, benign))

    def test_denial_marker_sets_do_not_drift_between_the_two_scripts(self) -> None:
        # The workflows sparse-check out only their own script, so a shared import is
        # impossible; pin the duplication instead of letting the copies diverge.
        self.assertEqual(
            tuple(self.rearm.ARM_DENIAL_MARKERS),
            tuple(self.auto.ARM_DENIAL_MARKERS),
            "both arm scripts must classify the same denial set",
        )

    def test_remediation_names_every_flippable_lever(self) -> None:
        for module in (self.rearm, self.auto):
            for token in REQUIRED_REMEDIATION_TOKENS:
                self.assertIn(
                    token,
                    module.ARM_REMEDIATION,
                    f"{module.PROGRAM}: the single ::error must name {token!r}",
                )

    def test_exit_semantics_never_return_zero_on_an_arm_failure(self) -> None:
        self.assertEqual(self.rearm.SweepOutcome().exit_code, 0)
        self.assertEqual(
            self.rearm.SweepOutcome(arm_failures=[(1, "denied")]).exit_code, 1
        )
        self.assertEqual(
            self.rearm.SweepOutcome(capability=self.rearm.CANNOT_ARM).exit_code, 1
        )
        self.assertEqual(self.auto.ArmOutcome().exit_code, 0)
        self.assertEqual(self.auto.ArmOutcome(arm_failures=[(1, "denied")]).exit_code, 1)
        self.assertEqual(
            self.auto.ArmOutcome(capability=self.auto.CANNOT_ARM).exit_code, 1
        )

    @staticmethod
    def _outcome_class(module: ModuleType):
        return getattr(module, "SweepOutcome", None) or module.ArmOutcome

    def test_both_outcomes_carry_sticky_transient_state(self) -> None:
        # #3766: the per-candidate transient must be RECORDED state, not an exception that
        # unwinds the run — an exception is what discarded the earlier collected failure.
        for module in (self.rearm, self.auto):
            outcome = self._outcome_class(module)()
            for attribute in (
                "transient_exhaustions",
                "sweep_transient",
                "hard_failed",
                "transient_detail",
            ):
                self.assertTrue(
                    hasattr(outcome, attribute),
                    f"{module.PROGRAM}: {attribute} is load-bearing for #3766",
                )

    def test_a_collected_failure_outranks_a_later_transient_exhaustion(self) -> None:
        for module in (self.rearm, self.auto):
            outcome_class = self._outcome_class(module)
            dominated = outcome_class(
                arm_failures=[(451, "unstable")],
                transient_exhaustions=[(452, "HTTP 504")],
            )
            self.assertTrue(dominated.hard_failed, module.PROGRAM)
            self.assertEqual(
                dominated.exit_code,
                1,
                f"{module.PROGRAM}: a LATER candidate's exhausted transient must never "
                "downgrade a COLLECTED arm failure to success (#3766)",
            )
            # ... while the lenient policy survives where it IS correct.
            transient_only = outcome_class(transient_exhaustions=[(452, "HTTP 504")])
            self.assertFalse(transient_only.hard_failed, module.PROGRAM)
            self.assertEqual(transient_only.exit_code, 0, module.PROGRAM)
            self.assertIsNotNone(transient_only.transient_detail, module.PROGRAM)

    def test_both_publish_the_outcome_on_the_runner(self) -> None:
        # The exit is computed at the END from this state, so it must be reachable after an
        # exception escapes run() — a local variable is exactly what #3766 lost.
        rearm_runner = self.rearm.RearmSweeper(
            "o/r", "main", gh=lambda _argv: "", log=lambda _line: None
        )
        auto_runner = self.auto.AutoArmer("o/r", "main", lambda _argv: "", lambda _l: None)
        for runner, module in ((rearm_runner, self.rearm), (auto_runner, self.auto)):
            self.assertIsInstance(
                runner.outcome, self._outcome_class(module), module.PROGRAM
            )

    def test_both_expose_an_end_of_run_precedence_function(self) -> None:
        self.assertTrue(callable(self.rearm.sweep_exit))
        self.assertTrue(callable(self.auto.arm_exit_code))
        for module in (self.rearm, self.auto):
            source = (REARM_PY if module is self.rearm else AUTO_ARM_PY).read_text(
                encoding="utf-8"
            )
            self.assertIn(
                "collected-failure > transient-exhaustion > clean",
                source,
                f"{module.PROGRAM}: the precedence rule must be documented in-source",
            )

    def test_capability_probe_query_reads_the_repository_setting(self) -> None:
        for module in (self.rearm, self.auto):
            self.assertIn("autoMergeAllowed", module.CAPABILITY_QUERY)

    def test_a_null_viewer_permission_never_blocks(self) -> None:
        # MEASURED: Repository.viewerPermission is null when authenticated as a GitHub App,
        # and the Actions GITHUB_TOKEN *is* an App installation token — so it can only ever
        # be diagnostics. PullRequest.viewerCanEnableAutoMerge is likewise unusable: it is
        # false for already-armed (#2521), queued (#3764), draft and merged PRs even under
        # an ADMIN user token. Gating on either would refuse legitimate arms. This pins the
        # only thing that matters behaviourally: a null viewerPermission with the setting
        # ON must still read as can-arm.
        for module in (self.rearm, self.auto):
            response = {
                "data": {
                    "repository": {
                        "autoMergeAllowed": True,
                        "viewerPermission": None,
                    },
                    "viewer": {"login": "github-actions[bot]"},
                }
            }
            verdict = self._probe_with(module, response)
            self.assertEqual(verdict.status, module.CAN_ARM, verdict)
            # ... and the setting being OFF must be decisive regardless.
            response["data"]["repository"]["autoMergeAllowed"] = False
            verdict = self._probe_with(module, response)
            self.assertEqual(verdict.status, module.CANNOT_ARM, verdict)
            self.assertIn("Allow auto-merge", verdict.detail)

    @staticmethod
    def _probe_with(module: ModuleType, response: dict):
        import json

        def fake_gh(_argv: list[str]) -> str:
            return json.dumps(response)

        runner = (
            module.RearmSweeper("o/r", "main", gh=fake_gh, log=lambda _line: None)
            if hasattr(module, "RearmSweeper")
            else module.AutoArmer("o/r", "main", fake_gh, lambda _line: None)
        )
        return runner.probe_arm_capability()

    def test_both_scripts_stay_import_free_of_each_other(self) -> None:
        for path in (REARM_PY, AUTO_ARM_PY):
            source = path.read_text(encoding="utf-8")
            self.assertNotIn("import rearm_sweeper", source, path)
            self.assertNotIn("import auto_arm", source, path)


class TestSelfTestsRunInCi(unittest.TestCase):
    """The self-tests are only worth writing if something runs them on every PR."""

    def test_docs_quality_runs_the_arm_policy_self_tests_and_this_wiring_suite(self) -> None:
        document = load(WORKFLOWS / "docs-quality.yml")
        gating = [
            run_of(step)
            for job_id, job in (document.get("jobs") or {}).items()
            for step in (job.get("steps") or [])
            if "advisory" not in str(job.get("name", job_id)).lower()
        ]
        # [OPUS-5] #4795: match a whole stripped LINE, not a substring of the blob. A
        # containment check passes against `… --self-test-DISABLED` / `… --self-test || true`
        # — the exact suffix-mutation shape that survived a containment pin elsewhere in
        # this repo — so the command must appear as its own complete command line.
        commands = {
            line.strip()
            for block in gating
            for line in block.splitlines()
            if line.strip()
        }
        for command in (
            # [OPUS-5] #4795: gh_retry.py was the ONLY one of the three arm-policy scripts
            # whose PR-time self-test was unpinned. docs-quality.yml did run it, so the
            # coverage looked complete — but deleting that single line would have gone
            # unnoticed, leaving every change to the transient classifier (the thing that
            # decides whether a platform blip reds main) validated by nothing until the
            # next cron. Pinning the file whose guard you are relying on is the point.
            "python3 scripts/gh_retry.py --self-test",
            "python3 scripts/rearm-sweeper.py --self-test",
            "python3 scripts/auto-arm.py --self-test",
            "python3 scripts/tests/test_arm_capability_wiring.py",
        ):
            self.assertIn(
                command,
                commands,
                f"docs-quality.yml must run `{command}` as a complete command line in a "
                "GATING job (found gating commands: "
                f"{sorted(c for c in commands if 'self-test' in c or 'test_arm' in c)})",
            )


# --------------------------------------------------------------------------------------
# [OPUS-5] #3776 — THE MISSING-SIBLING-IMPORT CLASS.
#
# auto-arm.yml runs on `pull_request`, so GitHub takes the WORKFLOW FILE — and therefore
# its `sparse-checkout` manifest — from the PR's own ref, while the checkout step takes the
# SCRIPT from `ref: default_branch`. #3766 added `import gh_retry` to scripts/auto-arm.py
# and `scripts/gh_retry.py` to that manifest in ONE commit: atomic on main, not atomic
# across refs. Every PR ref snapshotted earlier kept the one-entry manifest, so the runner
# paired the NEW script with a checkout that never materialised gh_retry.py:
#   ModuleNotFoundError: No module named 'gh_retry'  (#3434, run 30143852994)
# `arm reviewed PRs` is GATING, so ci-summary fail-fasted and no reviewed PR on a stale ref
# could merge — a missing RESILIENCE helper became a merge blocker.
#
# These suites pin BOTH halves of the fix:
#   * SURVIVABILITY (heals the already-stale refs) — each script must import and pass its
#     own self-test with gh_retry.py ABSENT, and the degraded path must still do the
#     load-bearing work (mark ready + issue the arm mutation), never quietly no-op.
#   * IMPOSSIBILITY (stops new skew) — no workflow may enumerate individual .py files whose
#     sibling imports are not also enumerated, and the arm workflows must sparse-check out
#     the whole `scripts` directory.
SCRIPTS_DIR = REPO_ROOT / "scripts"
ARM_SCRIPTS = (REARM_PY, AUTO_ARM_PY)


def run_without_gh_retry(
    script: Path, driver: str | None = None, *, with_release_guard: bool = True
):
    """Run ``script`` from a temp dir holding ONLY it, so `import gh_retry` cannot resolve.

    This reproduces the runner's state exactly: sys.path[0] is the script's own directory,
    and scripts/gh_retry.py was never checked out into it.

    [OPUS-5] #1135: ``with_release_guard`` (default True) ALSO copies
    scripts/release_pr_guard.py in, isolating exactly ONE variable — the missing
    gh_retry.py this class is about. That guard has the OPPOSITE degradation contract
    (its absence must stop every arm, not degrade one), so leaving it out here would
    conflate the two and make this class assert something it does not mean. The
    release-guard-absent case has its own class: TestMissingReleaseGuardArmsNothing.
    """
    env = {k: v for k, v in __import__("os").environ.items() if k != "PYTHONPATH"}
    with tempfile.TemporaryDirectory() as tmp:
        target = Path(tmp) / script.name
        shutil.copy2(script, target)
        if with_release_guard:
            shutil.copy2(script.parent / "release_pr_guard.py", Path(tmp))
        if (Path(tmp) / "gh_retry.py").exists():  # pragma: no cover - paranoia
            raise AssertionError("the isolation dir must NOT contain gh_retry.py")
        if driver is None:
            argv = [sys.executable, str(target), "--self-test"]
        else:
            (Path(tmp) / "driver.py").write_text(driver, encoding="utf-8")
            argv = [sys.executable, str(Path(tmp) / "driver.py"), target.name]
        return subprocess.run(
            argv, cwd=tmp, capture_output=True, text=True, check=False, env=env
        )


DRIVER_PRELUDE = textwrap.dedent(
    '''
    import importlib.util
    import sys
    from pathlib import Path

    path = Path(sys.argv[1]).resolve()
    spec = importlib.util.spec_from_file_location("under_test", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules["under_test"] = module
    spec.loader.exec_module(module)

    # The whole point: this driver must be running in the DEGRADED mode, not silently
    # picking up the real helper from somewhere on sys.path.
    assert module.GH_RETRY_DEGRADED is True, "expected degraded mode"
    assert module.gh_retry is module._DegradedGhRetry, module.gh_retry
    '''
)

# The degraded path must still MUTATE. "Gracefully degraded into doing nothing" is the
# failure mode these two drivers exist to catch: they assert the draft->ready mutation AND
# the arm mutation are really issued, and that the outcome counts the arm.
AUTO_ARM_DRIVER = DRIVER_PRELUDE + textwrap.dedent(
    """
    # Draft in the list snapshot, ready on the live refresh: exercises BOTH mutations
    # (gh pr ready, then the enablePullRequestAutoMerge CAS).
    fake, messages, outcome = module.exercise(
        module.fixture(draft=True), views=[module.fixture()]
    )
    assert outcome.armed == 1, (outcome, messages)
    assert not outcome.arm_failures, outcome.arm_failures
    assert any(call[:2] == ["pr", "ready"] for call in fake.calls), fake.calls
    arm = module.graphql_calls(fake)
    assert len(arm) == 1, fake.calls
    assert any("enablePullRequestAutoMerge" in str(a) for a in arm[0]), arm
    assert any("armed head" in m for m in messages), messages
    print("DEGRADED-MUTATION-OK")
    """
)

REARM_DRIVER = DRIVER_PRELUDE + textwrap.dedent(
    """
    fake, messages, outcome = module.exercise(module.fixture(4242))
    assert outcome.armed == 1, (outcome, messages)
    assert not outcome.arm_failures, outcome.arm_failures
    arm = module.arm_calls(fake)
    assert len(arm) == 1, fake.calls
    assert "--auto" in arm[0], arm
    print("DEGRADED-MUTATION-OK")
    """
)

DEGRADED_DRIVERS = {REARM_PY: REARM_DRIVER, AUTO_ARM_PY: AUTO_ARM_DRIVER}


class TestSurvivesMissingGhRetry(unittest.TestCase):
    """#3776: a missing resilience helper must cost RETRIES, never the arm."""

    def test_self_test_passes_with_gh_retry_absent(self) -> None:
        # THE REGRESSION TEST. Before the import guard this reproduced the live
        # ModuleNotFoundError from run 30143852994 verbatim.
        for script in ARM_SCRIPTS:
            with self.subTest(script=script.name):
                result = run_without_gh_retry(script)
                self.assertNotIn(
                    "ModuleNotFoundError: No module named 'gh_retry'",
                    result.stderr,
                    f"{script.name} must not abort when gh_retry.py was not checked out "
                    "(the #3434 outage); it must degrade to no-retry",
                )
                self.assertEqual(
                    0,
                    result.returncode,
                    f"{script.name} --self-test must pass without gh_retry.py\n"
                    f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}",
                )
                self.assertIn("self-test: PASS", result.stdout, result.stdout)

    def test_degraded_import_emits_exactly_one_loud_actionable_warning(self) -> None:
        for script in ARM_SCRIPTS:
            with self.subTest(script=script.name):
                out = run_without_gh_retry(script).stdout
                self.assertEqual(
                    1,
                    out.count("::warning title="),
                    f"{script.name}: exactly ONE ::warning, never one per call\n{out}",
                )
                for needle in (
                    "gh_retry.py",  # the missing file, by name
                    "sparse-checkout",  # the mechanism
                    "ONE-SHOT",  # what was actually lost
                    "REMEDY",  # what to do about it
                ):
                    self.assertIn(needle, out, f"{script.name}: warning must name {needle!r}")

    def test_degraded_path_still_performs_the_mutations(self) -> None:
        # NOT-A-NO-OP. Degrading into silence would be worse than crashing: the check
        # would go green while nothing was ever armed.
        for script in ARM_SCRIPTS:
            with self.subTest(script=script.name):
                result = run_without_gh_retry(script, driver=DEGRADED_DRIVERS[script])
                self.assertEqual(
                    0,
                    result.returncode,
                    f"{script.name} degraded arm path\nstdout:\n{result.stdout}\n"
                    f"stderr:\n{result.stderr}",
                )
                self.assertIn("DEGRADED-MUTATION-OK", result.stdout, result.stdout)


class TestMissingReleaseGuardArmsNothing(unittest.TestCase):
    """[OPUS-5] #1135: the OPPOSITE degradation contract to TestSurvivesMissingGhRetry.

    A missing `gh_retry.py` costs RETRIES and must never cost the arm (#3776). A missing
    `release_pr_guard.py` costs the ARM ITSELF and must: without it the sweep cannot prove
    any candidate is not the release-plz Release PR, and arming that PR cuts a `v*` tag
    and — once `publish = true` — publishes 37 crates to crates.io, irreversibly. A missed
    sweep is covered by the next cron; an unpublishable version is covered by nothing.

    These are the tests that go red if someone 'fixes' the fail-closed stub into a
    permissive one to make TestSurvivesMissingGhRetry pass more easily.
    """

    def test_arm_sweeps_refuse_every_candidate_without_the_guard(self) -> None:
        for script in ARM_SCRIPTS:
            with self.subTest(script=script.name):
                result = run_without_gh_retry(
                    script,
                    driver=DEGRADED_DRIVERS[script],
                    with_release_guard=False,
                )
                # The driver asserts `outcome.armed == 1`, so a NON-zero exit here is the
                # assertion that nothing was armed. Paired with
                # TestSurvivesMissingGhRetry, which runs the SAME driver WITH the guard
                # present and requires exit 0 — the two together prove the difference is
                # attributable to the guard and to nothing else.
                self.assertNotEqual(
                    0,
                    result.returncode,
                    f"{script.name} ARMED a PR with release_pr_guard.py absent. The "
                    "Release-PR exclusion could not be evaluated, so nothing may be "
                    "armed (#1135).\nstdout:\n" + result.stdout,
                )
                self.assertNotIn("DEGRADED-MUTATION-OK", result.stdout, result.stdout)

    def test_the_refusal_is_loud_and_actionable(self) -> None:
        for script in ARM_SCRIPTS:
            with self.subTest(script=script.name):
                out = run_without_gh_retry(
                    script, with_release_guard=False
                ).stdout
                self.assertIn("::error title=", out, out)
                for needle in ("release_pr_guard.py", "sparse-checkout", "#1135"):
                    self.assertIn(needle, out, f"{script.name}: must name {needle!r}")


class TestDegradedHelperContract(unittest.TestCase):
    """The stand-in is exercised directly — it must not be a permissive stub."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.rearm = load_module(REARM_PY, "rearm_sweeper_under_test")
        cls.auto = load_module(AUTO_ARM_PY, "auto_arm_under_test")

    def modules(self):
        return ((self.rearm, "rearm-sweeper"), (self.auto, "auto-arm"))

    def test_the_real_helper_is_used_when_it_is_importable(self) -> None:
        # Non-vacuity in the OTHER direction: a guard that always degraded would silently
        # delete #3759's transient tolerance from every scheduled run.
        for module, name in self.modules():
            self.assertFalse(module.GH_RETRY_DEGRADED, name)
            self.assertEqual("gh_retry", module.gh_retry.__name__, name)

    def test_degraded_read_only_guard_still_refuses_the_arm_mutation(self) -> None:
        for module, name in self.modules():
            degraded = module._DegradedGhRetry
            with self.assertRaises(degraded.GhRetryUsageError, msg=name):
                degraded.assert_read_only(
                    ["api", "graphql", "-f", f"query={module.ENABLE_AUTO_MERGE}"]
                    if hasattr(module, "ENABLE_AUTO_MERGE")
                    else ["pr", "merge", "1", "--auto"]
                )
            # ...and refuses a mutation whichever shape it takes.
            for refused in (
                ["pr", "merge", "1", "--auto", "--squash"],
                ["pr", "edit", "1", "--add-label", "review:pass"],
                ["api", "-X", "POST", "repos/o/r/issues"],
                ["api", "graphql", "--input", "body.json"],
                ["api", "graphql", "-f", "query=mutation{enablePullRequestAutoMerge}"],
            ):
                with self.assertRaises(degraded.GhRetryUsageError, msg=(name, refused)):
                    degraded.assert_read_only(refused)
            # The reads the sweeps actually issue must still be accepted, or the degraded
            # mode would be a different kind of brick.
            for allowed in (
                ["pr", "list", "--repo", "o/r", "--json", "number"],
                ["pr", "view", "1", "--repo", "o/r", "--json", "number"],
                ["api", "graphql", "-f", f"query={module.CAPABILITY_QUERY}"],
            ):
                degraded.assert_read_only(allowed)

    def test_degraded_read_failure_is_fatal_never_lenient(self) -> None:
        # GhTransientExhausted is what #3759 converts into ::warning + exit 0. With no
        # retries there is nothing to exhaust, so synthesising it here would turn a 403
        # into a false success. Fail closed instead.
        class _Failed:
            returncode = 1
            stdout = ""
            stderr = "HTTP 504: 504 Gateway Timeout"

        for module, name in self.modules():
            degraded = module._DegradedGhRetry
            with self.assertRaises(degraded.GhFatalError, msg=name) as caught:
                degraded.run_gh_read(
                    ["pr", "list", "--repo", "o/r"], run=lambda *_a, **_k: _Failed()
                )
            self.assertNotIsInstance(
                caught.exception, degraded.GhTransientExhausted, name
            )
            self.assertIn("NOT retried", str(caught.exception), name)

    def test_degraded_stand_ins_do_not_drift_between_the_two_scripts(self) -> None:
        # Same rule as the denial-marker pin: the workflows cannot share a helper, so the
        # duplication is pinned rather than left to rot.
        self.assertEqual(
            tuple(sorted(self.rearm._DegradedGhRetry.READ_SUBCOMMANDS)),
            tuple(sorted(self.auto._DegradedGhRetry.READ_SUBCOMMANDS)),
        )
        for name in ("assert_read_only", "run_gh_read"):
            self.assertTrue(callable(getattr(self.rearm._DegradedGhRetry, name)), name)
            self.assertTrue(callable(getattr(self.auto._DegradedGhRetry, name)), name)


class TestSparseCheckoutManifest(unittest.TestCase):
    """#3776 IMPOSSIBILITY half: a manifest that can go stale must not exist."""

    def test_arm_workflows_check_out_the_whole_scripts_directory(self) -> None:
        for name, (path, _script) in ARM_WORKFLOWS.items():
            for step in steps_of(load(path)):
                with_ = step.get("with") or {}
                if "sparse-checkout" not in with_:
                    continue
                pattern = str(with_["sparse-checkout"]).split()
                self.assertEqual(
                    ["scripts"],
                    pattern,
                    f"{name}: enumerate the DIRECTORY, not files — a file list must be "
                    "hand-updated for every new sibling import and is snapshotted per "
                    "PR ref (#3776)",
                )
                self.assertNotIn(
                    "sparse-checkout-cone-mode",
                    with_,
                    f"{name}: cone mode is what makes the bare `scripts` pattern "
                    "recursive; non-cone would reinterpret it as a gitignore pattern",
                )

    def test_no_script_shadows_a_stdlib_module(self) -> None:
        """The hazard the whole-directory checkout introduces, closed up front.

        `python3 scripts/auto-arm.py` puts scripts/ FIRST on sys.path. While the manifest
        listed two files that was harmless; now the entire directory is materialised, so a
        file named e.g. scripts/types.py would shadow the stdlib for every script run from
        there. Cheap to pin, silent and confusing to debug.
        """
        offenders = sorted(
            str(p.relative_to(REPO_ROOT))
            for p in SCRIPTS_DIR.rglob("*.py")
            if p.stem in sys.stdlib_module_names
        )
        self.assertEqual(
            [],
            offenders,
            "these scripts shadow a stdlib module and would break any sibling script "
            f"importing it: {offenders}",
        )

    def test_no_workflow_enumerates_a_python_file_without_its_sibling_imports(
        self,
    ) -> None:
        """The whole CLASS, over every workflow — not just the two arm ones.

        A `sparse-checkout` list that names individual .py files silently encodes that
        script's import graph. If a listed script imports a sibling under scripts/ that
        the list omits, the runner gets a script it cannot import: #3434 exactly.
        """
        local_modules = {p.stem: p for p in SCRIPTS_DIR.rglob("*.py")}
        offenders: list[str] = []
        for workflow in sorted((REPO_ROOT / ".github" / "workflows").glob("*.yml")):
            document = load(workflow)
            if not isinstance(document, dict):
                continue
            for step in steps_of(document):
                with_ = step.get("with") or {}
                raw = with_.get("sparse-checkout")
                if not raw:
                    continue
                listed = [entry.strip() for entry in str(raw).split() if entry.strip()]
                listed_stems = {Path(entry).stem for entry in listed}
                for entry in listed:
                    if not entry.endswith(".py"):
                        continue
                    script = REPO_ROOT / entry
                    if not script.is_file():
                        continue
                    tree = ast.parse(script.read_text(encoding="utf-8"))
                    imported: set[str] = set()
                    for node in ast.walk(tree):
                        if isinstance(node, ast.Import):
                            imported.update(a.name.split(".")[0] for a in node.names)
                        elif isinstance(node, ast.ImportFrom) and node.module:
                            imported.add(node.module.split(".")[0])
                    for sibling in sorted(imported & set(local_modules)):
                        if sibling not in listed_stems:
                            offenders.append(
                                f"{workflow.name}: sparse-checkout lists {entry} which "
                                f"imports sibling {sibling!r} "
                                f"({local_modules[sibling].relative_to(REPO_ROOT)}) that "
                                "the manifest omits — check out the directory instead"
                            )
        self.assertEqual([], offenders, "\n".join(offenders))


# --------------------------------------------------------------------------------------
# [OPUS-5] #3776 SECOND HALF — AN EVENT-TRIGGERED RUN JUDGES THE PR THAT TRIGGERED IT.
#
# `AutoArmer.run()` swept EVERY open review:pass PR in BOTH modes. #3766 then made the exit
# status STICKY over collected failures — correct for the cron sweep, catastrophic combined
# with whole-repo scope on a GATING check: ONE permanently un-armable PR reds
# `arm reviewed PRs` on every OTHER PR's label event.
#
# MEASURED blast radius (run 30144214363, 2026-07-25T04:33Z): sparq had exactly two
# review:pass PRs and the sweep reported `considered=2 armed=0 arm-failures=1` — #3434
# cannot be armed at all until the `workflows: write` question (#3777) is decided, so the
# gating arm check was poisoned for the whole repository by one PR.
#
# The fix is SCOPE, not leniency. Two halves are pinned below, and the DISCRIMINATION
# between them is the test:
#   * an UNRELATED PR's un-armability must be invisible to an event-mode run, and
#   * the TRIGGERING PR's own arm failure must still be a hard non-zero exit that no later
#     transient can discard (#3766's precedence, unchanged, within the narrower scope).
# Plus the two ways this could silently become a no-op: sweep mode must still cover the
# whole repository, and the WORKFLOW must actually plumb the triggering PR number through
# to the script (a production call site — the class a unit test on candidates() misses).
UNARMABLE = 3434  # modifies a workflow file; no `workflows: write` on the arm token
ARMABLE = 2521


def arm_run(module: ModuleType, *, scope_pr: int | None, mode: str, failing=UNARMABLE):
    """Replay the live shape: two review:pass PRs, one of them permanently un-armable."""
    fake = module.FakeGh(
        [module.fixture(number=ARMABLE), module.fixture(number=UNARMABLE)],
        mutation_errors={failing: module.WORKFLOWS_DENIAL} if failing else None,
    )
    lines: list[str] = []
    armer = module.AutoArmer(
        "sparq-org/sparq", "main", fake, lines.append, scope_pr=scope_pr
    )
    try:
        return fake, lines, module.run_sweep(armer, log=lines.append, mode=mode)
    except module.GhError as error:  # event-mode transient exhaustion raises
        return fake, lines, error


class TestEventModeIsPerPr(unittest.TestCase):
    """An event-triggered run must be accountable for the triggering PR and no other."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.auto = load_module(AUTO_ARM_PY, "auto_arm_under_test")

    def test_an_unarmable_unrelated_pr_does_not_fail_an_event_run(self) -> None:
        fake, lines, code = arm_run(self.auto, scope_pr=ARMABLE, mode="event")
        self.assertEqual(
            0,
            code,
            f"an event run for PR #{ARMABLE} must not fail because a DIFFERENT PR "
            f"(#{UNARMABLE}) cannot be armed — that is what poisoned the gating check "
            f"for the whole repo (#3776)\n" + "\n".join(lines),
        )
        self.assertTrue(any(f"PR #{ARMABLE}: armed head" in l for l in lines), lines)
        # Not merely excused — never even READ, so nothing about it can reach this outcome.
        self.assertEqual(
            [], [l for l in lines if str(UNARMABLE) in l], "the bystander must be untouched"
        )
        self.assertEqual(
            [], [c for c in fake.calls if str(UNARMABLE) in " ".join(c)], fake.calls
        )

    def test_the_triggering_prs_own_failure_is_still_fatal(self) -> None:
        # THE DISCRIMINATION. Narrowing WHICH PRs a run answers for must not weaken
        # accountability for the one it does: this is the half that keeps the check honest.
        fake, lines, code = arm_run(self.auto, scope_pr=UNARMABLE, mode="event")
        self.assertEqual(
            1,
            code,
            "the TRIGGERING PR's own arm failure must still red the run\n"
            + "\n".join(lines),
        )
        self.assertTrue(any(f"PR #{UNARMABLE}: arm-failed" in l for l in lines), lines)
        self.assertEqual(
            [], [l for l in lines if l.startswith("::warning")], "no lenient warning"
        )

    def test_a_later_transient_cannot_discard_the_triggering_prs_failure(self) -> None:
        # #3766's sticky precedence, INSIDE the narrower scope: the arm fails, then that
        # same PR's race diagnostic exhausts its bounded retries. Exit 1 must stand.
        module = self.auto
        fake = module.FakeGh(
            [module.fixture(number=UNARMABLE)],
            mutation_errors={UNARMABLE: module.WORKFLOWS_DENIAL},
        )
        views: list[list[str]] = []

        def diagnostic_exhausts(argv: list[str]) -> str:
            if argv[:2] == ["pr", "view"] and argv[2] == str(UNARMABLE):
                views.append(argv)
                if len(views) >= 3:  # candidates(), pre-arm refresh, then the diagnostic
                    raise module.gh_retry.GhTransientExhausted("HTTP 504 (3 attempts)")
            return fake(argv)

        lines: list[str] = []
        code = module.run_sweep(
            module.AutoArmer(
                "sparq-org/sparq",
                "main",
                fake,
                lines.append,
                gh_read=diagnostic_exhausts,
                scope_pr=UNARMABLE,
            ),
            log=lines.append,
            mode="event",
        )
        self.assertEqual(1, code, "\n".join(lines))
        self.assertEqual([], [l for l in lines if l.startswith("::warning")], lines)
        self.assertTrue(
            any("race diagnostic could not resolve" in l for l in lines), lines
        )

    def test_sweep_mode_still_covers_the_whole_repo(self) -> None:
        # Without this, the scoping fix could silently become "event mode does nothing and
        # nothing else covers the repository".
        _fake, lines, code = arm_run(
            self.auto, scope_pr=None, mode="sweep", failing=None
        )
        self.assertEqual(0, code, "\n".join(lines))
        self.assertEqual(
            2,
            sum("armed head" in l for l in lines),
            "the periodic sweep must still consider EVERY open review:pass PR\n"
            + "\n".join(lines),
        )
        self.assertTrue(
            any("considered=2" in l and "scope=all" in l for l in lines), lines
        )
        # ...and it stays ACCOUNTABLE for all of them: one failure still reds the sweep.
        _fake, lines, code = arm_run(self.auto, scope_pr=None, mode="sweep")
        self.assertEqual(1, code, "\n".join(lines))
        self.assertTrue(any(f"PR #{ARMABLE}: armed head" in l for l in lines), lines)
        self.assertTrue(any(f"PR #{UNARMABLE}: arm-failed" in l for l in lines), lines)

    def test_the_workflow_passes_the_triggering_pr_number_to_the_arm_step(self) -> None:
        """THE CALL SITE. Scoping the script is worthless if nothing supplies the number."""
        steps = steps_of(load(AUTO_ARM_YML))
        arm = [
            step
            for step in steps
            if "scripts/auto-arm.py" in run_of(step)
            and PROBE_FLAG not in run_of(step)
            and "--self-test" not in run_of(step)
        ]
        self.assertEqual(1, len(arm), "auto-arm.yml must have exactly one arming step")
        run, env = run_of(arm[0]), (arm[0].get("env") or {})
        match = re.search(r'--pr\s+"\$\{?([A-Z_]+)\}?"', run)
        self.assertIsNotNone(
            match,
            "the arming step must pass --pr \"$VAR\" — without it an event-mode run "
            "cannot know which PR it is accountable for (#3776)\n" + run,
        )
        variable = match.group(1)
        self.assertIn(variable, env, f"{variable} must be set in the step env")
        self.assertEqual(
            "${{ github.event.pull_request.number }}",
            str(env[variable]).strip(),
            f"{variable} must be EXACTLY the triggering PR's number: any fallback value "
            "would scope the periodic sweep too, and the sweep is the whole-repo backstop",
        )
        # The mode axis must survive alongside it — scope answers 'whose failure is this
        # run's business', mode answers 'how loud is a missed cycle' (#3759 finding 5).
        mode_match = re.search(r'--mode\s+"\$\{?([A-Z_]+)\}?"', run)
        self.assertIsNotNone(mode_match, run)
        self.assertIn("event", str(env[mode_match.group(1)]))
        self.assertIn("sweep", str(env[mode_match.group(1)]))

    def test_the_script_alone_can_scope_a_stale_ref(self) -> None:
        """The half that heals ALREADY-FROZEN PR refs.

        `ARM_PR` above is snapshotted per PR ref exactly like the sparse-checkout manifest
        that caused #3776, so a stale ref cannot pass it. The script always comes from the
        default branch, so it must be able to derive the number from the RUNNER's
        environment on its own — otherwise this fix would only reach rebased PRs.
        """
        resolve = self.auto.resolve_scope_pr
        self.assertEqual(ARMABLE, resolve("", {"GITHUB_REF": f"refs/pull/{ARMABLE}/merge"}))
        self.assertIsNone(resolve("", {"GITHUB_REF": "refs/heads/main"}))
        with tempfile.TemporaryDirectory() as tmp:
            payload = Path(tmp) / "event.json"
            payload.write_text('{"pull_request": {"number": 909}}', encoding="utf-8")
            self.assertEqual(
                909,
                resolve("", {"GITHUB_EVENT_PATH": str(payload)}),
                "the event payload is the most specific environment source",
            )
        # An explicit --pr always wins, and a typo is loud rather than a whole-repo sweep.
        self.assertEqual(42, resolve("42", {"GITHUB_REF": f"refs/pull/{ARMABLE}/merge"}))
        with self.assertRaises(ValueError):
            resolve("not-a-number", {})

    def test_the_scoping_seam_is_documented_in_source(self) -> None:
        source = AUTO_ARM_PY.read_text(encoding="utf-8")
        for needle in (
            "scope_pr",
            "def candidates",
            "def resolve_scope_pr",
            # The distinction the maintainer asked to keep visible in the code: scoping is
            # a CORRECTNESS fix; de-gating `arm reviewed PRs` is a POLICY decision (#3777).
            "policy decision about what may block a merge",
        ):
            self.assertIn(needle, source, f"auto-arm.py must keep {needle!r}")



# --------------------------------------------------------------------------------------
# [OPUS-5] #4548 — THE STUCK-ARM PHASE, pinned at the YAML SEAM.
#
# Measured here repeatedly: every uncaught mutant in this repo's recent rounds lived in the
# workflow, not the Python. A wiring assertion once stayed green because the step's COMMENT
# named the file it searched for; a `paths:`-filtered workflow never ran the suite guarding
# its own headline mutant. So the pins below assert against the PARSED document (yaml drops
# comments structurally — proved by `test_the_harness_is_comment_blind`) and against the
# SCRIPT's real argparse surface, so deleting either half reds.
STUCK_PHASE_FLAG = "--phase stuck-arm"
STUCK_CAP_FLAG = "--max-stuck-actions"


class TestStuckArmWiring(unittest.TestCase):
    """The stuck-arm sweep must be REACHED, BOUNDED, and PERMITTED — or it is decoration."""

    def setUp(self) -> None:
        self.document = load(REARM_YML)
        self.steps = steps_of(self.document)

    def _stuck_indexes(self) -> list[int]:
        return [
            index
            for index, step in enumerate(self.steps)
            if STUCK_PHASE_FLAG in " ".join(run_of(step).split())
        ]

    def test_the_harness_is_comment_blind(self) -> None:
        """The tripwire for the measured false-green: a COMMENT must not satisfy a pin."""
        commented = yaml.safe_load(
            "jobs:\n  j:\n    steps:\n"
            f"      # runs rearm-sweeper.py {STUCK_PHASE_FLAG} {STUCK_CAP_FLAG} 5\n"
            "      - name: decoy\n        run: echo nothing-here\n"
        )
        blob = "\n".join(run_of(step) for step in steps_of(commented))
        self.assertNotIn(STUCK_PHASE_FLAG, blob)
        self.assertNotIn(STUCK_CAP_FLAG, blob)
        # ...and the same token in a real `run:` IS seen, so the check is not vacuous.
        live = yaml.safe_load(
            "jobs:\n  j:\n    steps:\n"
            f"      - name: real\n        run: python3 x.py {STUCK_PHASE_FLAG}\n"
        )
        self.assertIn(
            STUCK_PHASE_FLAG, "\n".join(run_of(step) for step in steps_of(live))
        )

    def test_the_stuck_arm_phase_is_actually_invoked_exactly_once(self) -> None:
        indexes = self._stuck_indexes()
        self.assertEqual(
            len(indexes),
            1,
            f"rearm-sweeper.yml must run `{STUCK_PHASE_FLAG}` exactly once; a phase that "
            "is never invoked classifies nothing, and two invocations double-act",
        )
        self.assertIn(
            "scripts/rearm-sweeper.py",
            run_of(self.steps[indexes[0]]),
            "the stuck-arm flag must be passed to rearm-sweeper.py itself",
        )

    def test_it_runs_after_the_rearm_step(self) -> None:
        """Order is policy: a PR re-armed seconds ago must be inside the grace window."""
        stuck = self._stuck_indexes()[0]
        rearm = [
            index
            for index, step in enumerate(self.steps)
            if "scripts/rearm-sweeper.py" in run_of(step)
            and PROBE_FLAG not in run_of(step)
            and "--self-test" not in run_of(step)
            and STUCK_PHASE_FLAG not in " ".join(run_of(step).split())
        ]
        self.assertTrue(rearm, "the re-arm step must still exist")
        self.assertGreater(
            stuck, max(rearm), "the stuck-arm phase must run AFTER the re-arm phase"
        )

    def test_the_per_tick_bound_is_actually_passed(self) -> None:
        """A congestion bound that is never supplied is not a bound.

        This repo has a measured congestion-collapse mode, so the cap has to be on the
        command line, not merely available as a default.
        """
        run = " ".join(run_of(self.steps[self._stuck_indexes()[0]]).split())
        self.assertIn(STUCK_CAP_FLAG, run)
        value = run.split(STUCK_CAP_FLAG, 1)[1].split()[0]
        self.assertTrue(value.isdigit(), f"{STUCK_CAP_FLAG} needs a numeric cap, got {value!r}")
        self.assertGreaterEqual(int(value), 1)
        self.assertLessEqual(
            int(value), 10, "a per-tick cap above 10 re-opens the congestion-collapse mode"
        )

    def test_the_live_step_is_not_stuck_in_dry_run(self) -> None:
        """A sweep permanently in --dry-run reports beautifully and repairs nothing.

        The flag exists so the census can be taken against the live repository before
        remediation is switched on; leaving it in the scheduled step would recreate exactly
        the invisible-no-exit state this phase was built to remove.
        """
        run = " ".join(run_of(self.steps[self._stuck_indexes()[0]]).split())
        self.assertNotIn("--dry-run", run)

    def test_it_uses_the_same_token_expression_as_the_probe(self) -> None:
        stuck = self.steps[self._stuck_indexes()[0]]
        probe = next(
            step for step in self.steps if PROBE_FLAG in run_of(step)
        )
        self.assertEqual(
            (stuck.get("env") or {}).get("GH_TOKEN"),
            (probe.get("env") or {}).get("GH_TOKEN"),
            "a stuck-arm phase on a different token than the probe attests capability it "
            "does not have",
        )

    def test_the_scopes_its_mutations_need_are_granted(self) -> None:
        """Classify-then-403 is the failure mode this pin exists for.

        [GPT-6 Astra] #6438 keeps existing workflow grants. The preferred App's
        Actions grant governs reruns; the fallback has no Actions write and fails
        closed. These declarations never elevate the minted installation token.
        """
        permissions = self.document.get("permissions") or {}
        for scope in ("contents", "pull-requests", "checks", "issues"):
            self.assertEqual(
                permissions.get(scope),
                "write",
                f"the stuck-arm phase cannot remediate without `{scope}: write`",
            )


# --------------------------------------------------------------------------------------
# [OPUS-5] #4642 — THE YAML SEAM ITSELF, lifted from #4400's docs-quality guard (which was
# hardened into this exact shape after an earlier round found `|| true` on either leg
# surviving every other pin in its class).
#
# Everything above this line tests the Python, or tests that the YAML NAMES the Python.
# Neither can see the seam that decides whether the Python RUNS AT ALL. RE-DERIVED here by
# applying each shape to the real rearm-sweeper.yml and running the suite as it stood
# before this class existed — all four survived GREEN:
#   1. `continue-on-error: true` on the JOB
#   2. `continue-on-error: true` on the STEP
#   3. `if: false` on the step
#   4. `|| true` appended to the run line
# A fifth, `if: false` on the JOB, falls out of the same read and is pinned with them.
#
# The reason is structural, and it is this estate's most-repeated defect class: a step
# cannot red its own neutering (`continue-on-error` discards the exit status that would
# report it) and a job cannot red its own skipping (`ci_summary_gate._PASSING` is
# `("success", "skipped", "neutral")`). Something OUTSIDE the run has to witness that it
# ran — so these checks read the PARSED document and never any result of executing it.
# This workflow is not itself a gate (schedule / workflow_dispatch only), which makes the
# seam MORE dangerous, not less: a neutered cron produces no red anywhere, it just
# silently stops sweeping.
#
# `seam_findings` is a pure function of the document precisely so that
# `test_the_guard_reds_on_each_swallow_shape` can feed it a MUTATED copy of the real
# workflow and require the corresponding finding. A seam guard nobody has watched go red
# is the thing a seam guard exists to prevent.
REARM_SCRIPT = "scripts/rearm-sweeper.py"

# Every `if:` permitted on a step that runs the sweep, keyed by step name. FAIL-CLOSED in
# both directions: a step that runs the sweep must appear here, and its `if:` must match
# EXACTLY. So `if: false`, a plausible-looking `github.event_name` guard, and a brand-new
# unreviewed step all red, rather than inheriting silence from a permissive predicate.
SEAM_STEP_IFS: dict[str, str | None] = {
    "Self-test policy": None,
    "Probe arm capability (one loud error, never per-PR)": None,
    "Re-arm dropped reviewed PRs": None,
    # #4642: the ONE vetted condition. Neither clause can be false while the sweep is
    # needed — see the rationale block at the step itself. `always()` is deliberately NOT
    # what is written there: this step mutates the repository.
    "Sweep armed-but-unmergeable PRs to a counted terminal state": (
        "${{ !cancelled() && steps.probe.outcome == 'success' }}"
    ),
}

# A line invoking the sweep must BE the whole command. `||`, `;`, `|` and `&` each decide
# the exit status the runner sees, so none may follow it — the anchored bare-call shape
# test_banned_terminology.py uses, and the one a substring match cannot enforce
# (`… --self-test || true` still "contains" `… --self-test`).
BARE_SWEEP_LINE = re.compile(r"^[ \t]*python3 +" + re.escape(REARM_SCRIPT) + r"[^|;&]*$")


class Finding(NamedTuple):
    kind: str
    message: str


def _invokes_sweep(line: str) -> bool:
    stripped = line.strip()
    return stripped.startswith("python3") and REARM_SCRIPT in stripped


def sweep_steps(document: dict) -> list[tuple[str, dict, dict]]:
    """(job_id, job, step) for every step whose `run` invokes the sweep script."""
    return [
        (job_id, job, step)
        for job_id, job in (document.get("jobs") or {}).items()
        for step in (job.get("steps") or [])
        if any(_invokes_sweep(line) for line in run_of(step).splitlines())
    ]


def seam_findings(document: dict) -> list[Finding]:
    """Every way this document could run the sweep and not report its failure."""
    hosts = sweep_steps(document)
    if not hosts:
        return [Finding("absent", f"no step runs {REARM_SCRIPT} at all")]
    findings: list[Finding] = []
    for job_id, job, step in hosts:
        name = str(step.get("name") or "<unnamed>")
        if job.get("continue-on-error") not in (None, False):
            findings.append(Finding(
                "job-continue-on-error",
                f"job {job_id!r} hosting {name!r} is continue-on-error, so a failed sweep "
                "reports the job green",
            ))
        if step.get("continue-on-error") not in (None, False):
            findings.append(Finding(
                "step-continue-on-error",
                f"step {name!r} is continue-on-error, so it cannot red its own failure",
            ))
        if job.get("if") is not None:
            findings.append(Finding(
                "if",
                f"job {job_id!r} hosting {name!r} carries `if: {job.get('if')!r}`; a job "
                "cannot red its own skipping",
            ))
        if name not in SEAM_STEP_IFS:
            findings.append(Finding(
                "undeclared",
                f"step {name!r} runs the sweep but is not declared in SEAM_STEP_IFS — "
                "decide and record whether it may carry an `if:`",
            ))
        elif step.get("if") != SEAM_STEP_IFS[name]:
            findings.append(Finding(
                "if",
                f"step {name!r} carries `if: {step.get('if')!r}`, not the vetted "
                f"{SEAM_STEP_IFS[name]!r}; an `if:` is the cheapest way to make a sweep "
                "vacuous, and a skipped step is not a failed one",
            ))
        run = run_of(step)
        for line in run.splitlines():
            if _invokes_sweep(line) and not BARE_SWEEP_LINE.match(line):
                findings.append(Finding(
                    "discard",
                    f"step {name!r} does not invoke the sweep as a bare command, so its "
                    f"exit code can be discarded by what follows: {line.strip()!r}",
                ))
        if "set +e" in run:
            findings.append(Finding(
                "discard", f"step {name!r} disables errexit with `set +e`"
            ))
    return findings


class TestTheYamlSeamIsGating(unittest.TestCase):
    """The wiring above is only worth anything if a failure of it can be seen."""

    def setUp(self) -> None:
        self.document = load(REARM_YML)

    def _kinds(self, *kinds: str) -> list[str]:
        return [f.message for f in seam_findings(self.document) if f.kind in kinds]

    def test_the_sweep_is_reached_by_at_least_one_step(self) -> None:
        self.assertTrue(sweep_steps(self.document), f"nothing runs {REARM_SCRIPT}")

    def test_no_sweep_step_can_swallow_its_own_failure(self) -> None:
        self.assertEqual(self._kinds("job-continue-on-error", "step-continue-on-error"), [])

    def test_no_sweep_step_is_conditionally_skipped_by_an_unvetted_if(self) -> None:
        self.assertEqual(self._kinds("if", "undeclared"), [])

    def test_no_sweep_step_discards_its_exit_code(self) -> None:
        self.assertEqual(self._kinds("discard"), [])

    def test_every_declared_step_still_exists(self) -> None:
        """A rename must not leave a dead allowance behind, silently vetting nothing."""
        live = {str(step.get("name")) for _job_id, _job, step in sweep_steps(self.document)}
        self.assertEqual(
            sorted(set(SEAM_STEP_IFS) - live),
            [],
            "SEAM_STEP_IFS names steps that rearm-sweeper.yml no longer has",
        )

    def test_the_stuck_arm_terminal_is_not_skipped_when_its_predecessor_fails(self) -> None:
        """#4642's measured gap: 5 of the 200 most recent runs failed, and both retained
        failures were the step directly in FRONT of this one. With no `if:`, GitHub skips
        a step whenever an earlier one failed — so the terminal was unavailable exactly on
        the ticks that needed it. `!cancelled()` is what makes a failed predecessor still
        run it; the probe clause is what keeps the "fail loud once" contract."""
        _job_id, _job, step = sweep_steps(self.document)[-1]
        condition = str(step.get("if") or "")
        self.assertIn("!cancelled()", condition)
        self.assertNotIn("success()", condition)
        self.assertIn(
            "steps.probe.outcome",
            condition,
            "the probe clause must read `outcome` (the pre-continue-on-error result), so "
            "it cannot be laundered green by marking the probe continue-on-error",
        )
        probe = next(
            step for _j, _job, step in sweep_steps(self.document)
            if PROBE_FLAG in run_of(step)
        )
        self.assertEqual(
            probe.get("id"), "probe", "the `if:` above references a step id that must exist"
        )

    def test_the_guard_reds_on_each_swallow_shape(self) -> None:
        """THE VACUITY GUARD. Each mutation is applied to a deep copy of the REAL workflow
        and must produce a finding of its OWN kind — not merely some finding, which would
        pass even if one check were doing all the work."""
        def stuck(document: dict) -> tuple[dict, dict]:
            _job_id, job, step = sweep_steps(document)[-1]
            return job, step

        mutants: tuple[tuple[str, str, object], ...] = (
            ("job-continue-on-error", "continue-on-error on the JOB",
             lambda job, step: job.__setitem__("continue-on-error", True)),
            ("step-continue-on-error", "continue-on-error on the STEP",
             lambda job, step: step.__setitem__("continue-on-error", True)),
            ("if", "`if: false` on the step",
             lambda job, step: step.__setitem__("if", False)),
            ("if", "a plausible-looking event guard on the step",
             lambda job, step: step.__setitem__("if", "github.event_name == 'schedule'")),
            ("if", "`if: false` on the job",
             lambda job, step: job.__setitem__("if", False)),
            ("discard", "`|| true` appended to the run line",
             lambda job, step: step.__setitem__("run", run_of(step) + " || true")),
            ("discard", "`set +e` before the invocation",
             lambda job, step: step.__setitem__("run", "set +e\n" + run_of(step))),
            ("undeclared", "an undeclared new step running the sweep",
             lambda job, step: job["steps"].append(
                 {"name": "sneak", "run": f"python3 {REARM_SCRIPT} --phase stuck-arm"})),
        )
        for kind, label, mutate in mutants:
            with self.subTest(shape=label):
                document = copy.deepcopy(self.document)
                mutate(*stuck(document))
                self.assertIn(
                    kind,
                    [f.kind for f in seam_findings(document)],
                    f"{label} survives the seam guard",
                )
        # That the guard is not simply always-red is what the three tests above assert,
        # against the unmutated document — so it is deliberately not repeated here.


class TestStuckArmScriptContract(unittest.TestCase):
    """Cross-file pin: the YAML above is only meaningful if the SCRIPT still honours it."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.rearm = load_module(REARM_PY, "rearm_sweeper_stuck_under_test")

    def test_the_script_accepts_the_flags_the_workflow_passes(self) -> None:
        source = REARM_PY.read_text(encoding="utf-8")
        for needle in ('"--phase"', '"stuck-arm"', '"--max-stuck-actions"'):
            self.assertIn(needle, source, f"rearm-sweeper.py must define {needle}")
        self.assertTrue(hasattr(self.rearm, "StuckArmSweeper"))
        self.assertTrue(hasattr(self.rearm, "stuck_arm_exit"))

    def test_the_stuck_self_test_is_reachable_from_dash_dash_self_test(self) -> None:
        """THE VACUITY GUARD.

        Everything else in this file assumes the stuck-arm suite runs. If `self_test()`
        stops calling `stuck_self_test()`, that whole suite becomes dead code that still
        reports PASS — the exact shape of a green-but-vacuous gate. Asserted on the AST so
        a mention in a comment or a docstring cannot satisfy it.
        """
        tree = ast.parse(REARM_PY.read_text(encoding="utf-8"))
        self_test = next(
            node
            for node in tree.body
            if isinstance(node, ast.FunctionDef) and node.name == "self_test"
        )
        called = {
            node.func.id
            for node in ast.walk(self_test)
            if isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
        }
        self.assertIn(
            "stuck_self_test",
            called,
            "self_test() must CALL stuck_self_test(); otherwise the stuck-arm suite never "
            "runs and every pin in this file is vacuous",
        )

    def test_the_two_gate_names_are_distinct_and_matched_exactly(self) -> None:
        """registry #761 in miniature: `gate` is a strict prefix of `gate, draft-tier`."""
        self.assertNotEqual(self.rearm.GATE_CHECK_NAME, self.rearm.DRAFT_GATE_CHECK_NAME)
        self.assertTrue(
            self.rearm.DRAFT_GATE_CHECK_NAME.startswith(self.rearm.GATE_CHECK_NAME)
        )
        pages = [
            {
                "total_count": 1,
                "check_runs": [
                    {
                        "name": self.rearm.DRAFT_GATE_CHECK_NAME,
                        "status": "completed",
                        "conclusion": "success",
                        "started_at": "2026-07-27T00:00:00Z",
                        "id": 1,
                    }
                ],
            }
        ]
        self.assertEqual(
            self.rearm.resolve_gate(pages, is_draft=False), self.rearm.GATE_MISSING
        )
        self.assertEqual(
            self.rearm.resolve_gate(pages, is_draft=True), self.rearm.GATE_SUCCESS
        )

    def test_every_class_is_routed(self) -> None:
        """The enum is closed in BOTH directions — no class without an action."""
        actions = self.rearm.CLASS_ACTIONS
        self.assertTrue(actions)
        self.assertEqual(
            set(actions.values()) - {
                self.rearm.ACTION_NONE, self.rearm.ACTION_PARK,
                self.rearm.ACTION_ROUTE_FIX, self.rearm.ACTION_REBASE,
                self.rearm.ACTION_RETRIGGER,
            },
            set(),
        )



# [GPT-6 Astra] #6438: exercise the production rerun path with distinct server IDs.
# The existing workflow runs THIS suite; these are not an uncalled fixture appendix.
class ActionsRerunFixture:
    def __init__(self, module):
        self.m = module
        self.clock = module._iso_epoch("2026-09-06T12:00:00Z")
        self.viewer = {"login": "sparq-orchestrator[bot]"}
        self.actor = dict(id="BOT_4300853", login=self.viewer["login"], __typename="Bot")
        self.raw = module.live_pr(6360, labels=("review:pass",), head="a" * 40)
        self.check = dict(module.check_run("gate", "cancelled", ident=101),
            head_sha="a" * 40, app={"slug": "github-actions"},
            details_url="https://github.com/sparq-org/sparq/actions/runs/303/job/202")
        self.run = dict(id=303, workflow_id=404, run_attempt=1, event="pull_request",
            path=".github/workflows/ci-summary.yml", status="completed", conclusion="cancelled",
            head_sha="a" * 40, repository={"full_name": "sparq-org/sparq"},
            pull_requests=[{"number": 6360, "head": {"sha": "a" * 40}}])
        self.job = dict(id=202, run_id=303, head_sha="a" * 40, name="gate",
            status="completed", conclusion="cancelled",
            check_run_url="https://api.github.com/repos/sparq-org/sparq/check-runs/101")
        self.comments = []
        self.calls = []
        self.after_claim = lambda: None
        self.claim_reply = None
        self.post_error = False
        self.history_extra = 0
        self.history_truncated = False
        self.latest_extra = []
        self.jobs_extra = 0
        self.logs = []
        self.sweeper = self.new_sweeper()
        self.original = self.sweeper.live_pull(6360)
        self.calls.clear()

    def new_sweeper(self):
        return self.m.StuckArmSweeper("sparq-org/sparq", "main", gh=self,
            log=self.logs.append, now=lambda: self.clock)

    def __call__(self, argv):
        self.calls.append(list(argv))
        if argv[:2] == ["pr", "list"]:
            return json.dumps([self.raw])
        if argv[:2] == ["api", "graphql"]:
            query = next(x for x in argv if x.startswith("query="))
            if "pullRequests(states:OPEN)" in query:
                return json.dumps({"data": {"repository": {"pullRequests": {"totalCount": 1}}}})
            if "viewer{" in query:
                return json.dumps({"data": {"viewer": self.viewer, "repository": {"pullRequest": {
                    "comments": {"totalCount": len(self.comments) + self.history_extra,
                        "pageInfo": {"hasPreviousPage": self.history_truncated}, "nodes": self.comments}}}}})
            return json.dumps({"data": {"repository": {"pullRequest": self.raw}}})
        if argv[:3] == ["api", "-X", "POST"]:
            if argv[3].endswith("/comments"):
                body = next(x.removeprefix("body=") for x in argv if x.startswith("body="))
                self.comments.append(dict(databaseId=505, body=body, author=copy.deepcopy(self.actor)))
                self.raw["updatedAt"] = "2026-09-06T12:00:00Z"
                self.after_claim()
                return self.claim_reply if self.claim_reply is not None else json.dumps({"id": 505})
            if argv[3] == "repos/sparq-org/sparq/actions/jobs/202/rerun":
                if self.post_error:
                    raise self.m.GhError("Resource not accessible by integration (HTTP 403)")
                return "{}"
            raise AssertionError(f"unexpected mutation: {argv}")
        path = argv[-1]
        if path == f"users/{self.viewer['login']}":
            return json.dumps(dict(login=self.actor["login"],node_id=self.actor["id"],type="Bot"))
        if "/commits/" in path:
            return json.dumps(self.m.check_pages([self.check]))
        if "/actions/workflows/ci-summary.yml/runs?" in path:
            rows = [self.run] + self.latest_extra
            return json.dumps([{"total_count": len(rows), "workflow_runs": rows}])
        if "/attempts/1/jobs?" in path:
            return json.dumps([{"total_count": 1 + self.jobs_extra, "jobs": [self.job]}])
        if path == "repos/sparq-org/sparq/check-runs/101":
            return json.dumps(self.check)
        if path == "repos/sparq-org/sparq/actions/runs/303":
            return json.dumps(self.run)
        raise AssertionError(f"unexpected read: {argv}")

    def posts(self):
        return [c for c in self.calls if c[:3] == ["api", "-X", "POST"]]

    def reruns(self):
        return [c for c in self.posts() if c[3].endswith("/rerun")]

    def drive(self):
        self.sweeper.retrigger(self.original, 101, "fixture")


class TestCancelledActionsRecovery(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.m = load_module(REARM_PY, "rerun_rearm")

    def setUp(self):
        def poison(*args, **kwargs):
            raise AssertionError("unexpected process/network access in rerun fixture")
        for target, name in ((subprocess, "run"), (subprocess, "Popen"),
                             (socket, "create_connection"), (socket.socket, "connect")):
            patcher = patch.object(target, name, poison)
            patcher.start()
            self.addCleanup(patcher.stop)
        self.f = ActionsRerunFixture(self.m)

    def denied(self):
        with self.assertRaises(self.m.GhError):
            self.f.drive()
        self.assertEqual(self.f.reruns(), [])

    def test_distinct_check_job_and_run_ids_reach_actions_only(self):
        self.assertEqual(self.f.sweeper.run(), 0)
        self.assertEqual([c[3] for c in self.f.posts()], [
            "repos/sparq-org/sparq/issues/6360/comments",
            "repos/sparq-org/sparq/actions/jobs/202/rerun"])
        claim = self.m.parse_rerun_claim(self.f.comments[0]["body"])
        self.assertEqual((claim["check"], claim["job"], claim["run"], claim["attempt"]), (101, 202, 303, 1))
        self.assertFalse(any("/rerequest" in arg for call in self.f.calls for arg in call))

    def test_fallback_identity_is_refused_before_any_claim_or_rerun(self):
        # [GPT-6 Astra] Opus B1: valid bot metadata cannot grant the explicitly
        # unprivileged fallback a claim. Keep the real App happy path above.
        self.f.viewer = {"login": "github-actions[bot]"}
        self.f.actor = dict(id="BOT_ACTIONS", login="github-actions[bot]", __typename="Bot")
        self.f.post_error = True  # model the fallback's known Actions denial
        self.assertEqual(self.f.sweeper.run(), 1)
        self.assertEqual(self.f.posts(), [])
        self.assertEqual(self.f.comments, [])
        self.assertTrue(any("github-actions[bot]" in line and "no actions:write" in line
                            for line in self.f.logs), self.f.logs)

    def test_rerun_claim_cannot_shadow_or_satisfy_a_park_receipt(self):
        # [GPT-6 Astra] Opus B2: exercise the actual park parser/evaluator, with
        # a new claim appearing AFTER the park in a complete comment history.
        park = self.m.stuck_comment(self.f.original, "gate-failed", self.m.GATE_FAILURE,
                                   "fixture park", self.f.clock)
        self.f.comments = [dict(databaseId=501, body=park, author=self.f.actor)]
        self.f.drive()
        claim_body = self.f.comments[-1]["body"]
        self.assertEqual(len(self.f.reruns()), 1)
        self.assertNotIn(self.m.STUCK_MARKER, claim_body)
        self.assertNotIn(self.m.RECEIPT_OPEN, claim_body)
        self.assertIsNone(self.m.parse_stuck_receipt(claim_body))
        self.assertIsNone(self.m.parse_rerun_claim(park))
        expected_park = self.m.parse_stuck_receipt(park)
        for history in (park + "\n\n" + claim_body, claim_body + "\n\n" + park):
            selected = self.m.parse_stuck_receipt(history)
            self.assertEqual(selected, expected_park)
            self.assertTrue(self.m.unpark_satisfied(selected, self.f.original, self.m.GATE_SUCCESS))
            self.assertFalse(self.m.unpark_satisfied(selected, self.f.original, self.m.GATE_FAILURE))
        claim = self.m.parse_rerun_claim(claim_body)
        self.assertFalse(self.m.unpark_satisfied(claim, self.f.original, self.m.GATE_SUCCESS))

    def test_claim_history_cannot_change_the_existing_rearm_path(self):
        self.f.drive()
        claim_history = copy.deepcopy(self.f.comments)
        for held in (False, True):
            with self.subTest(held=held):
                labels = ("review:pass", "needs:user") if held else ("review:pass",)
                dropped = self.m.fixture(6360, labels=labels, armed=False)
                dropped["comments"] = {"nodes": claim_history}
                fake, _messages, outcome = self.m.exercise(dropped)
                self.assertEqual(outcome.exit_code, 0)
                self.assertEqual(len(self.m.arm_calls(fake)), 0 if held else 1)
                # There is no production comment fetch/selector in re-arm; the
                # separate unpark evaluator remains a human-only tool today.
                queries = [arg for call in fake.calls for arg in call if arg.startswith("query=")]
                self.assertFalse(any("comments(" in query for query in queries))

    def test_crlf_claim_readback_preserves_integrity_and_attempt_dedupe(self):
        def normalize_server_body():
            self.f.comments[-1]["body"] = self.f.comments[-1]["body"].replace("\n", "\r\n")
        self.f.after_claim = normalize_server_body
        self.assertEqual(self.f.sweeper.run(), 0)
        self.assertEqual(len(self.f.reruns()), 1)
        self.f.clock += 1800
        self.assertEqual(self.f.new_sweeper().run(), 1)
        self.assertEqual(len(self.f.reruns()), 1)
        self.assertEqual(len(self.f.comments), 1)

    def test_changed_head_draft_review_hold_queue_and_grace_never_claim(self):
        changes = [dict(headRefOid="b" * 40), dict(isDraft=True), dict(state="CLOSED"),
            dict(autoMergeRequest=None), dict(baseRefName="other"), dict(mergeable="UNKNOWN"),
            dict(mergeQueueEntry={"id": "queued"}), dict(updatedAt="2026-09-06T12:00:00Z"),
            dict(labels={"nodes": [], "pageInfo": {"hasNextPage": False}}),
            dict(labels={"nodes": [{"name": "review:pass"}, {"name": "needs:user"}], "pageInfo": {"hasNextPage": False}}),
            dict(reviewThreads={"totalCount": 1,"nodes": [{"isResolved": False}], "pageInfo": {"hasNextPage": False}}),
            dict(labels={"nodes": [{"name":"review:pass"}], "pageInfo":{"hasNextPage":True}}),
            dict(reviewThreads={"totalCount":0,"nodes":[],"pageInfo":{"hasNextPage":True}})]
        for change in changes:
            with self.subTest(change=change):
                self.f = ActionsRerunFixture(self.m)
                self.f.raw.update(change)
                self.denied()
                self.assertEqual(self.f.posts(), [])

    def test_explicit_6049_hold_survives_even_cancelled_metadata(self):
        self.f.raw["number"] = 6049
        self.f.original = self.m.replace(self.f.original, number=6049)
        self.f.run["pull_requests"][0]["number"] = 6049
        self.denied()
        self.assertEqual(self.f.posts(), [])

    def test_live_waiting_requested_and_action_required_are_not_cancelled(self):
        for target in ("check", "job", "run"):
            for status, conclusion in (("queued",None), ("in_progress",None), ("waiting",None),
                    ("requested",None), ("completed","action_required"), ("completed","success"),
                    ("completed","failure"), ("completed","stale")):
                with self.subTest(target=target, status=status, conclusion=conclusion):
                    self.f = ActionsRerunFixture(self.m)
                    getattr(self.f,target).update(status=status,conclusion=conclusion)
                    self.denied()
                    self.assertEqual(self.f.posts(), [])

    def test_wrong_app_url_job_run_head_workflow_event_or_attempt(self):
        cases = [("check", "app", {"slug":"other"}),
            ("check","details_url","https://github.com/other/repo/actions/runs/303/job/202"),
            ("check","head_sha","b"*40), ("check","name","gate, draft-tier"),
            ("job","check_run_url","https://api.github.com/repos/sparq-org/sparq/check-runs/999"),
            ("job","id",999), ("job","run_id",999), ("job","head_sha","b"*40),
            ("job","name","other"), ("run","head_sha","b"*40),
            ("run","path",".github/workflows/other.yml"),
            ("run","repository",{"full_name":"other/repo"}), ("run","pull_requests",[]),
            ("run","event","merge_group"), ("run","event","schedule"), ("run","event","workflow_dispatch"),
            ("run","run_attempt",2), ("run","run_attempt",True)]
        for target,key,value in cases:
            with self.subTest(target=target,key=key,value=value):
                self.f = ActionsRerunFixture(self.m)
                getattr(self.f,target)[key] = value
                self.denied()
                self.assertEqual(self.f.posts(), [])

    def test_latest_workflow_supersedes_cancelled_check_and_short_attempt_denies(self):
        for status,conclusion in (("waiting",None),("completed","cancelled")):
            self.f = ActionsRerunFixture(self.m)
            self.f.latest_extra = [dict(self.f.run,id=304,status=status,conclusion=conclusion)]
            self.denied()
            self.assertEqual(self.f.posts(), [])
        self.f = ActionsRerunFixture(self.m)
        self.f.jobs_extra = 1
        self.denied()
        self.assertEqual(self.f.posts(), [])

    def test_stale_metadata_after_claim_prevents_rerun(self):
        for target,key,value in (("raw","headRefOid","b"*40), ("raw","mergeQueueEntry",{"id":"q"}),
                ("run","run_attempt",2), ("run","status","waiting"), ("job","status","in_progress"),
                ("check","conclusion","success"),
                ("raw","labels",{"nodes":[{"name":"review:needs"}],"pageInfo":{"hasNextPage":False}})):
            with self.subTest(target=target,key=key):
                self.f = ActionsRerunFixture(self.m)
                self.f.after_claim = lambda: getattr(self.f,target).__setitem__(key,value)
                self.denied()
                self.assertEqual(len(self.f.posts()), 1)

    def test_claim_blocks_repeat_within_and_across_ticks_after_rejection(self):
        self.f.post_error = True
        self.assertEqual(self.f.sweeper.run(), 1)
        self.assertEqual(len(self.f.reruns()), 1)
        self.f.clock += 1800  # beyond the comment's grace, same server attempt
        self.assertEqual(self.f.sweeper.run(), 2)  # same instance retains sticky errors
        self.assertEqual(self.f.new_sweeper().run(), 1)
        self.assertEqual(len(self.f.reruns()), 1)
        self.assertEqual(len(self.f.comments), 1)

    def test_successful_but_not_yet_visible_attempt_stays_claimed(self):
        self.f.drive()
        self.f.clock += 1800
        self.assertEqual(self.f.new_sweeper().run(), 1)
        self.assertEqual(len(self.f.reruns()), 1)

    def test_receipts_must_be_complete_unquoted_and_authenticated(self):
        claim = self.f.sweeper.rerun_target(self.f.original,101)
        valid_body = self.m.StuckArmSweeper.rerun_claim_body(claim)
        old_body = self.m.StuckArmSweeper.rerun_claim_body(dict(claim,head="b"*40))
        for body, author in ((valid_body,dict(self.f.actor,id="OTHER")),
                (valid_body,dict(self.f.actor,login="someone")),
                (old_body,dict(self.f.actor,id="OTHER")),
                (old_body,dict(self.f.actor,login="someone")),
                (old_body,dict(self.f.actor,__typename="User")),
                ("> " + valid_body,self.f.actor), ("```\n"+valid_body+"\n```",self.f.actor),
                (valid_body + "\n",self.f.actor), (valid_body.replace("\n", "\r"),self.f.actor),
                (old_body.replace('"head":"'+'b'*40+'"','"head":"invalid"'),self.f.actor),
                (valid_body.replace('"attempt":1','"attempt":"bad"'),self.f.actor),
                (valid_body.replace('"repo":"sparq-org/sparq"','"repo":"other/repo"'),self.f.actor),
                (valid_body.replace("-->",""),self.f.actor),
                (valid_body,self.f.actor)):
            with self.subTest(body=body,author=author):
                self.f = ActionsRerunFixture(self.m)
                self.f.comments=[dict(databaseId=501,body=body,author=author)]
                self.denied()
                self.assertEqual(self.f.posts(), [])

    def test_authenticated_receipt_on_an_old_head_does_not_claim_new_head(self):
        claim = self.f.sweeper.rerun_target(self.f.original,101)
        self.f.comments = [dict(databaseId=501,author=self.f.actor,
            body=self.m.StuckArmSweeper.rerun_claim_body(dict(claim,head="b"*40)))]
        self.f.drive()
        self.assertEqual(len(self.f.reruns()),1)

    def test_same_attempt_is_claimed_even_if_job_or_check_id_changes(self):
        claim = self.f.sweeper.rerun_target(self.f.original,101)
        self.f.comments = [dict(databaseId=501,author=self.f.actor,
            body=self.m.StuckArmSweeper.rerun_claim_body(dict(claim,job=999,check=998)))]
        self.denied()
        self.assertEqual(self.f.posts(),[])

    def test_claim_is_not_review_evidence_or_a_verdict_bridge_trigger(self):
        claim = self.f.sweeper.rerun_target(self.f.original,101)
        body = self.m.StuckArmSweeper.rerun_claim_body(claim)
        bridge_tests = load_module(REPO_ROOT / "scripts/tests/test_verdict_bridge.py", "claim_bridge_tests")
        condition = load(WORKFLOWS / "verdict-bridge.yml")["jobs"]["bridge"]["if"]
        event = bridge_tests.payload("issue_comment", issue={"number":6360,"pull_request":{}}, comment={"body":body})
        self.assertFalse(bridge_tests.evaluate_if(condition,event))
        self.assertIsNone(bridge_tests.vb.trailing_verdict(body))
        # Positive control: the same production condition admits a real verdict.
        event["github"]["event"]["comment"]["body"] = "VERDICT: pass"
        self.assertTrue(bridge_tests.evaluate_if(condition,event))

    def test_truncated_history_or_nonbot_authentication_never_claims(self):
        for attr,value in (("history_truncated",True),("history_extra",1),
                ("viewer",{"id":"HUMAN","login":"jeswr","__typename":"User"})):
            self.f = ActionsRerunFixture(self.m)
            setattr(self.f,attr,value)
            self.denied()
            self.assertEqual(self.f.posts(), [])

    def test_claim_response_or_readback_uncertainty_never_posts_rerun(self):
        for reply in ("bad-json", "{}", '{"id":"505"}', '{"id":999}'):
            self.f = ActionsRerunFixture(self.m)
            self.f.claim_reply=reply
            self.denied()
            self.assertEqual(len(self.f.posts()),1)

    def test_per_tick_cap_and_dry_run_preserve_zero_posts(self):
        self.f.sweeper.limits=self.m.StuckLimits(max_actions=0)
        self.assertEqual(self.f.sweeper.run(),0)
        self.assertEqual(self.f.sweeper.deferred,1)
        self.assertEqual(self.f.posts(),[])
        self.f.sweeper=self.f.new_sweeper()
        self.f.sweeper.dry_run=True
        self.assertEqual(self.f.sweeper.run(),0)
        self.assertEqual(self.f.posts(),[])


if __name__ == "__main__":
    unittest.main(verbosity=2)
