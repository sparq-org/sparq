#!/usr/bin/env python3
"""Tests for the crates.io publishing protections (issues #1135, #2552).

[OPUS-5] 🤖 SPARQ agent.

WHY THIS FILE EXISTS AS A SEPARATE SUITE
========================================
Most of the protections live at seams that no other gate covers:

1. **The YAML seam.** `.github/workflows/release-plz.yml` and `release.yml` run only on
   `push: main` / a `v*` tag push, so neither EVER registers on a PR's `gate` aggregator —
   a defect in either surfaces for the first time at release time, when a crates.io
   publish cannot be undone. This suite therefore
   parses those workflows **structurally** (`yaml.safe_load`, then indexing into
   `jobs.<id>.steps[i]`) rather than by substring. Measured in this repo: 18/18 Python
   mutants died while every uncaught mutant lived in a workflow `if:` / step / call-site.
   A `"--enforce" in text` assertion does not catch `if: false`; `steps[i]["if"]` does.

2. **The arming seam.** `scripts/release_pr_guard.py` is consulted by five scripts. This
   suite asserts each one actually calls it — deleting the call from any of them reds a
   named test here, in addition to that script's own self-test.

Every assertion below is written to DISCRIMINATE: for each guard there is a paired case
that exercises the same code path and expects the OPPOSITE outcome, so a fixture cannot
satisfy both the right and the wrong reading.

Run: python3 scripts/tests/test_release_publish_guard.py
"""

from __future__ import annotations

import datetime as dt
import importlib.util
import json
import os
import re
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

# PyYAML is REQUIRED, never optional: skipping on ImportError would make this whole
# suite silently vacuous, which is the exact failure mode it exists to prevent.
import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = REPO_ROOT / "scripts"
RELEASE_PLZ_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "release-plz.yml"
GUARD_SCRIPT_REL = "scripts/release-interval-guard.py"
RELEASE_JOB_ID = "release-plz-release"

# The SECOND place a release can start (issue #2552): a `v*` tag push fires release.yml
# directly, never touching release-plz. Its `setup` job is where the guard runs, because
# every other job in that workflow depends on it.
RELEASE_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "release.yml"
RELEASE_SETUP_JOB_ID = "setup"
PUBLISH_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "publish.yml"

# The guard step's `run:` is pinned EXACTLY, not by substring. Four mutants applied to
# this step survived a substring/identity reading (reported on PR #4192):
#   run: … --enforce --repo-root . || true      -> `"--enforce" in run` still passes
#   run: exit 0; … --enforce --repo-root .      -> ditto
#   continue-on-error: 'true'                   -> `!= True` still passes (it is a str)
#   continue-on-error: ${{ true }}              -> ditto, and Actions evaluates it TRUE
# Equality plus absence closes both readings. If this command is ever legitimately
# changed, update this constant in the same commit — deliberately.
EXPECTED_GUARD_RUN = "python3 scripts/release-interval-guard.py --enforce --repo-root ."

# The tag-push guard (issue #2552), pinned by the same reasoning plus one mutant unique to
# it: DROPPING `--released-tag` leaves a step that runs, passes and guards nothing, because
# on a tag push `v<workspace version>` is already tagged and the guard reads that as an
# already-released no-op. The tag is passed through the environment, never interpolated
# into the command, so a tag name can never be spliced into the shell.
EXPECTED_TAG_PUSH_GUARD_RUN = (
    "python3 scripts/release-interval-guard.py --enforce --repo-root . "
    '--released-tag "${RELEASED_TAG}"'
)

# Shell constructs that can discard the guard's non-zero exit status, or run something
# before it that exits first. `runs-on: ubuntu-*` executes `run:` under `bash -e`, so a
# bare newline cannot swallow a failure — but `||`, `;`, `&` and a pipe all can.
_EXIT_STATUS_SWALLOWING = ("||", ";", "&", "|")


def _load(name: str, filename: str):
    """Import a scripts/*.py module by path (the directory is not a package)."""
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / filename)
    assert spec and spec.loader, filename
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


sys.path.insert(0, str(SCRIPTS))
release_pr_guard = _load("release_pr_guard", "release_pr_guard.py")
interval_guard = _load("release_interval_guard", "release-interval-guard.py")


class TestWorkspaceTagIsCreatedOnce(unittest.TestCase):
    """The locked workspace has one public tag, even though 37 crates release together."""

    def test_only_the_dependency_final_anchor_enables_git_tags(self) -> None:
        # release-plz processes every package independently. If the workspace default is
        # enabled, all 37 packages try to create the same `v{{ version }}` tag. [GPT-5.6]
        config = tomllib.loads(
            (REPO_ROOT / "release-plz.toml").read_text(encoding="utf-8")
        )
        workspace = config["workspace"]
        self.assertFalse(workspace.get("git_tag_enable"))
        self.assertEqual(workspace.get("git_tag_name"), "v{{ version }}")

        anchors = [
            package["name"]
            for package in config.get("package", [])
            if package.get("git_tag_enable") is True
        ]
        self.assertEqual(
            anchors,
            ["sparq-cli"],
            "exactly the dependency-final sparq-cli package must create the shared tag",
        )


class TestCrateAttestationPackagingFailsClosed(unittest.TestCase):
    """A missing `.crate` must stop provenance generation, not merely warn."""

    def test_packaging_collects_failures_and_refuses_incomplete_output(self) -> None:
        workflow = yaml.safe_load(PUBLISH_WORKFLOW.read_text(encoding="utf-8"))
        steps = workflow["jobs"]["crates"]["steps"]
        matches = [step for step in steps if step.get("name") == "Package publishable crates"]
        self.assertEqual(len(matches), 1)

        step = matches[0]
        self.assertNotIn("continue-on-error", step)
        run = step["run"]
        self.assertIn("cargo metadata --no-deps --format-version 1", run)
        self.assertIn("select(.publish != [])", run)
        self.assertIn("failures=()", run)
        self.assertIn('failures+=("$pkg")', run)
        self.assertIn('if [ "${#failures[@]}" -ne 0 ]; then', run)
        self.assertIn('if [ "$packaged" -ne "$expected" ]; then', run)
        self.assertGreaterEqual(run.count("exit 1"), 2)


def _workflow() -> dict:
    return yaml.safe_load(RELEASE_PLZ_WORKFLOW.read_text(encoding="utf-8"))


def _release_job_steps() -> list[dict]:
    jobs = _workflow()["jobs"]
    assert RELEASE_JOB_ID in jobs, (
        f"{RELEASE_PLZ_WORKFLOW.name} has no `{RELEASE_JOB_ID}` job — the guard's host "
        "job was renamed or removed"
    )
    return jobs[RELEASE_JOB_ID]["steps"]


def _guard_step_index(steps: list[dict]) -> int:
    matches = [
        i
        for i, step in enumerate(steps)
        if GUARD_SCRIPT_REL in str(step.get("run") or "")
    ]
    assert matches, (
        f"no step in `{RELEASE_JOB_ID}` runs {GUARD_SCRIPT_REL} — the #1135 "
        "publish-cadence guard step was DELETED from the release path"
    )
    assert len(matches) == 1, f"expected exactly one guard step, found {matches}"
    return matches[0]


def _release_plz_step_index(steps: list[dict]) -> int:
    matches = [
        i
        for i, step in enumerate(steps)
        if str(step.get("uses") or "").startswith("release-plz/action@")
    ]
    assert matches, (
        f"no `release-plz/action@…` step in `{RELEASE_JOB_ID}` — the release path this "
        "guard protects has moved; re-point the test"
    )
    return min(matches)


# ===================================================================== THE YAML SEAM
class TestReleaseWorkflowStructure(unittest.TestCase):
    """Structural (not textual) assertions over .github/workflows/release-plz.yml.

    Mutants each of these kills, verified by hand-editing the workflow and re-running:
      * delete the guard step               -> test_guard_step_exists
      * `if: false` on the guard step       -> test_guard_step_is_unconditional
      * `continue-on-error: true`           -> test_guard_step_is_not_continue_on_error
      * `continue-on-error: 'true'`         -> test_guard_step_is_not_continue_on_error
      * `continue-on-error: ${{ true }}`    -> test_guard_step_is_not_continue_on_error
      * move it after release-plz/action    -> test_guard_runs_before_release_plz
      * `--dry-run` instead of `--enforce`  -> test_guard_step_enforces
      * `… --enforce --repo-root . || true` -> test_guard_step_enforces AND
                                               test_guard_step_cannot_swallow_its_exit_status
      * `exit 0; … --enforce --repo-root .` -> both of the same two
    """

    def setUp(self) -> None:
        self.steps = _release_job_steps()

    def test_guard_step_exists(self) -> None:
        self.assertIsInstance(_guard_step_index(self.steps), int)

    def test_guard_step_is_unconditional(self) -> None:
        step = self.steps[_guard_step_index(self.steps)]
        # `if:` PARSED, not grepped: `if: false`, `if: ${{ false }}` and any other
        # condition would each silently skip the guard while leaving the step text intact.
        self.assertNotIn(
            "if",
            step,
            "the #1135 publish-cadence guard step carries an `if:` condition. It must be "
            "unconditional — a skipped guard is an absent guard, and the job it protects "
            "publishes irreversibly.",
        )

    def test_guard_step_is_not_continue_on_error(self) -> None:
        step = self.steps[_guard_step_index(self.steps)]
        # ABSENCE, not `!= True`. `continue-on-error: 'true'` and
        # `continue-on-error: ${{ true }}` both load as STRINGS, so an identity or
        # `is True` check passes while GitHub Actions evaluates them as true and the
        # release proceeds past a refusal. A `${{ … }}` expression cannot be evaluated
        # statically at all, so anything other than absence is refused here: the only
        # value that is provably not truthy at run time is no value.
        self.assertNotIn(
            "continue-on-error",
            step,
            "the #1135 publish-cadence guard step carries `continue-on-error: "
            f"{step.get('continue-on-error')!r}`. Any value — `true`, the string "
            "`'true'`, or a `${{ … }}` expression — turns its refusal into an advisory "
            "warning and the release publishes anyway. This step must carry no "
            "`continue-on-error` key at all.",
        )

    def test_guard_step_cannot_swallow_its_exit_status(self) -> None:
        # `… --enforce --repo-root . || true` and `exit 0; … --enforce …` both satisfy
        # a `"--enforce" in run` assertion while discarding the refusal. Independent of
        # the equality check below, so relaxing that one does not silently reopen this.
        run = str(self.steps[_guard_step_index(self.steps)]["run"])
        for token in _EXIT_STATUS_SWALLOWING:
            self.assertNotIn(
                token,
                run,
                f"the guard step's `run:` contains {token!r} ({run!r}). A shell "
                "operator can discard the guard's non-zero exit status (`|| true`) or "
                "short-circuit it (`exit 0;`), leaving the step present and the "
                "publish unguarded. Keep it a single bare command.",
            )

    def test_guard_runs_before_release_plz(self) -> None:
        guard = _guard_step_index(self.steps)
        release = _release_plz_step_index(self.steps)
        self.assertLess(
            guard,
            release,
            "the publish-cadence guard runs AFTER `release-plz/action` — by then the tag "
            "is cut and (with publish = true) the crates are already on crates.io.",
        )

    def test_guard_step_enforces(self) -> None:
        run = str(self.steps[_guard_step_index(self.steps)]["run"])
        # EQUALITY, not containment. A containment assertion is satisfied by
        # `python3 … --enforce --repo-root . || true`, which swallows the refusal, and
        # by `exit 0; python3 … --enforce …`, which never reaches it. Pinning the whole
        # command is the only reading that admits exactly one program.
        self.assertEqual(
            run.strip(),
            EXPECTED_GUARD_RUN,
            "the #1135 publish-cadence guard step's `run:` is not the exact expected "
            f"command.\n  expected: {EXPECTED_GUARD_RUN!r}\n  found:    {run!r}\n"
            "--dry-run always exits 0; a trailing `|| true` discards the refusal; a "
            "leading `exit 0;` skips it. If this command changed on purpose, update "
            "EXPECTED_GUARD_RUN in the same commit.",
        )

    def test_release_job_checks_out_full_history(self) -> None:
        # PRECONDITION for the guard: on a shallow checkout the tag list is not
        # authoritative, and the guard (correctly) refuses. Losing fetch-depth: 0 would
        # wedge every release rather than admit one — but it must still be pinned, or a
        # future 'fix' would be to relax the shallow check instead.
        checkout = next(
            step
            for step in self.steps
            if str(step.get("uses") or "").startswith("actions/checkout@")
        )
        self.assertEqual(
            checkout.get("with", {}).get("fetch-depth"),
            0,
            "the release job must check out with fetch-depth: 0 so the `v*` tag list is "
            "authoritative for the #1135 cadence guard.",
        )

    def test_release_plz_action_is_sha_pinned(self) -> None:
        uses = self.steps[_release_plz_step_index(self.steps)]["uses"]
        ref = uses.split("@", 1)[1].split(" ")[0]
        self.assertRegex(
            ref,
            r"^[0-9a-f]{40}$",
            f"release-plz/action must be SHA-pinned, got {ref!r}",
        )

    def test_workflow_never_runs_on_pull_request(self) -> None:
        # If this ever gains a pull_request trigger, a PR could reach the release path.
        triggers = _workflow()[True] if True in _workflow() else _workflow()["on"]
        self.assertNotIn("pull_request", triggers)
        self.assertNotIn("pull_request_target", triggers)


# ============================================ THE TAG-PUSH SEAM (issue #2552)
class TestTagPushReleaseIsCadenceGuarded(unittest.TestCase):
    """`.github/workflows/release.yml` — the OTHER way a release starts.

    release-plz.yml only covers releases that go through the Release PR. The runbook's
    canonical instruction is "push a `v*` tag", which fires release.yml directly and used
    to be entirely uncadenced. The guard now runs in that workflow's `setup` job — the
    one every other job depends on — so a refusal stops the archives, the SBOM/VEX, the
    GitHub Release and the ghcr image.

    Mutants each of these kills (verified by hand-editing the workflow and re-running):
      * delete the guard step                  -> test_guard_step_exists
      * `if: false` (or any `if:` at all)      -> test_guard_step_is_unconditional
      * `continue-on-error: true` (any form)   -> test_guard_step_is_not_continue_on_error
      * `--dry-run` instead of `--enforce`     -> test_guard_step_enforces
      * drop `--released-tag`                  -> test_guard_step_enforces (VACUITY: on a
        tag push the version is already tagged, so without it the guard always ALLOWs)
      * `… || true` / `exit 0; …`              -> test_guard_cannot_swallow_its_exit_status
      * lose `fetch-depth: 0`                  -> test_setup_job_checks_out_full_history
    """

    def setUp(self) -> None:
        self.workflow = yaml.safe_load(RELEASE_WORKFLOW.read_text(encoding="utf-8"))
        self.steps = self.workflow["jobs"][RELEASE_SETUP_JOB_ID]["steps"]

    def _guard(self) -> dict:
        matches = [
            step
            for step in self.steps
            if GUARD_SCRIPT_REL in str(step.get("run") or "")
        ]
        self.assertEqual(
            len(matches),
            1,
            f"expected exactly ONE step in `{RELEASE_SETUP_JOB_ID}` running "
            f"{GUARD_SCRIPT_REL}, found {len(matches)} — the #2552 cadence guard on the "
            "tag-push release path was deleted or duplicated",
        )
        return matches[0]

    def test_guard_step_exists(self) -> None:
        self.assertIsInstance(self._guard(), dict)

    def test_guard_step_enforces(self) -> None:
        # EQUALITY, not containment — the same reasoning as EXPECTED_GUARD_RUN above, plus
        # one more mutant unique to this seam: dropping `--released-tag` leaves a step that
        # runs, passes, and guards NOTHING (the pushed tag is `v<workspace version>`, which
        # the guard reads as an already-released no-op).
        self.assertEqual(
            str(self._guard()["run"]).strip(),
            EXPECTED_TAG_PUSH_GUARD_RUN,
            "the #2552 tag-push cadence guard's `run:` is not the exact expected command."
            f"\n  expected: {EXPECTED_TAG_PUSH_GUARD_RUN!r}"
            f"\n  found:    {str(self._guard()['run'])!r}\n"
            "If this changed on purpose, update EXPECTED_TAG_PUSH_GUARD_RUN in the same "
            "commit — deliberately.",
        )

    def test_guard_passes_the_resolved_version_as_the_released_tag(self) -> None:
        # The tag to exclude must be the SAME version string every other job names assets
        # with. Threading a different value (e.g. github.ref_name, which is a BRANCH on
        # dispatch) would exclude the wrong tag — or no tag at all, which is vacuity again.
        self.assertEqual(
            (self._guard().get("env") or {}).get("RELEASED_TAG"),
            "${{ steps.v.outputs.version }}",
            "the guard must exclude the version `setup` resolved, not some other ref",
        )

    def test_guard_step_is_unconditional(self) -> None:
        # PARSED, not grepped: `if: false` / `if: ${{ false }}` would leave the step text
        # intact while never running it. There is no condition worth admitting here —
        # a `workflow_dispatch` build creates a GitHub Release too, and publish.yml's
        # `npm` job runs on ANY `release` event, so exempting dispatch reopens the hole.
        self.assertNotIn(
            "if",
            self._guard(),
            "the #2552 cadence guard carries an `if:` condition. A skipped guard is an "
            "absent guard, and every trigger of this workflow ends in a published "
            "GitHub Release (which publish.yml then acts on).",
        )

    def test_guard_step_is_not_continue_on_error(self) -> None:
        # ABSENCE, not `!= True`: `'true'` and `${{ true }}` both load as strings while
        # Actions evaluates them truthy, turning a refusal into an advisory warning.
        self.assertNotIn(
            "continue-on-error",
            self._guard(),
            "the #2552 cadence guard must carry no `continue-on-error` key at all — any "
            "value lets the release proceed past a refusal.",
        )

    def test_guard_cannot_swallow_its_exit_status(self) -> None:
        run = str(self._guard()["run"])
        for token in _EXIT_STATUS_SWALLOWING:
            self.assertNotIn(
                token,
                run,
                f"the tag-push guard's `run:` contains {token!r} ({run!r}) — a shell "
                "operator can discard its non-zero exit status or short-circuit it, "
                "leaving the step present and the release unguarded.",
            )

    def test_setup_job_checks_out_full_history(self) -> None:
        # PRECONDITION: on a shallow checkout the `v*` tag list is not authoritative and
        # the guard (correctly) refuses. Pinned so a future "fix" for a wedged release is
        # not to relax the shallow check.
        checkout = next(
            step
            for step in self.steps
            if str(step.get("uses") or "").startswith("actions/checkout@")
        )
        self.assertEqual(
            (checkout.get("with") or {}).get("fetch-depth"),
            0,
            f"`{RELEASE_SETUP_JOB_ID}` must check out with fetch-depth: 0 so the `v*` "
            "tag list is authoritative for the #2552 cadence guard.",
        )

    def test_every_release_job_depends_on_the_guarded_setup_job(self) -> None:
        # The guard only bounds what `setup` gates. A job that does not depend on it —
        # transitively — would build/publish regardless of the verdict.
        jobs = self.workflow["jobs"]
        deps = {
            job_id: (
                [spec["needs"]]
                if isinstance(spec.get("needs"), str)
                else list(spec.get("needs") or [])
            )
            for job_id, spec in jobs.items()
        }

        def reaches_setup(job_id: str, seen: frozenset = frozenset()) -> bool:
            if job_id in seen:
                return False
            return any(
                dep == RELEASE_SETUP_JOB_ID or reaches_setup(dep, seen | {job_id})
                for dep in deps[job_id]
            )

        unguarded = sorted(
            job_id
            for job_id in jobs
            if job_id != RELEASE_SETUP_JOB_ID and not reaches_setup(job_id)
        )
        self.assertEqual(
            unguarded,
            [],
            f"{unguarded} in {RELEASE_WORKFLOW.name} do not depend on "
            f"`{RELEASE_SETUP_JOB_ID}`, so the #2552 publish-cadence guard cannot stop "
            "them. Add `needs: setup` (or a dependency that reaches it).",
        )


class TestReleaseCarriesTheExperimentalZkCaveat(unittest.TestCase):
    """Issue #2552: v0.1.0 ships WITHOUT the external ZK review, so every release must
    carry the experimental caveat in its own notes — the one surface a downloader who
    never opens the repo still reads."""

    def test_release_body_states_the_zk_mpc_scaffolds_are_unaudited(self) -> None:
        workflow = yaml.safe_load(RELEASE_WORKFLOW.read_text(encoding="utf-8"))
        bodies = [
            str((step.get("with") or {}).get("body") or "")
            for step in workflow["jobs"]["release"]["steps"]
        ]
        body = "\n".join(bodies)
        for fragment in (
            "Experimental",
            "sparq-zk",
            "sparq-mpc",
            "no external accredited cryptographer",
            "SECURITY.md",
        ):
            self.assertIn(
                fragment,
                body,
                f"the GitHub Release body no longer mentions {fragment!r}. The release "
                "ships ahead of the external ZK review (#2552) on the strength of that "
                "caveat; removing it makes the release notes overclaim by omission.",
            )


class TestArmWorkflowsSelfTestTheGuard(unittest.TestCase):
    """Both arm sweeps must run the guard's self-test before arming anything.

    Their scripts come from the DEFAULT branch at cron time, so a regression in the
    shared predicate would otherwise first be observed by arming the Release PR.
    """

    def _run_block(self, workflow_name: str, job_id: str) -> str:
        data = yaml.safe_load(
            (REPO_ROOT / ".github" / "workflows" / workflow_name).read_text("utf-8")
        )
        steps = data["jobs"][job_id]["steps"]
        return "\n".join(str(step.get("run") or "") for step in steps)

    def test_auto_arm_self_tests_the_release_guard(self) -> None:
        self.assertIn(
            "scripts/release_pr_guard.py --self-test",
            self._run_block("auto-arm.yml", "arm"),
        )

    def test_rearm_sweeper_self_tests_the_release_guard(self) -> None:
        self.assertIn(
            "scripts/release_pr_guard.py --self-test",
            self._run_block("rearm-sweeper.yml", "sweep"),
        )

    def test_docs_quality_gates_the_publishing_protections(self) -> None:
        # This suite must itself be invoked by a GATING job, or every assertion above is
        # dead code. docs-quality's job name contains neither "advisory" nor
        # "informational", so ci-summary discovers it as gating.
        data = yaml.safe_load(
            (REPO_ROOT / ".github" / "workflows" / "docs-quality.yml").read_text("utf-8")
        )
        runs = "\n".join(
            str(step.get("run") or "")
            for job in data["jobs"].values()
            for step in job.get("steps", [])
        )
        self.assertIn("scripts/tests/test_release_publish_guard.py", runs)
        self.assertIn("scripts/release-interval-guard.py --self-test", runs)


# ============================================================== THE RELEASE-PR EXCLUSION
class TestEveryArmingPathConsultsTheGuard(unittest.TestCase):
    """The enumerated arming paths each call release_pr_guard.

    Deleting the call from any one of them reds the named test here. The scripts' OWN
    self-tests also go red (proved by mutation in the PR body) — this is the second,
    cross-file net, so a path added later without the guard is visible.
    """

    PATHS = {
        "auto-arm.py": "arm_block_reason",
        "rearm-sweeper.py": "arm_block_reason",
        "check-pr-arm-base.py": "arm_block_reason",
        "batch-merge.py": "arm_block_reason",
        "pr-backlog.py": "arm_block_reason",
    }

    def test_each_arming_path_imports_and_calls_the_guard(self) -> None:
        # AST, not substring. MEASURED: the first draft of this test asserted
        # `"release_pr_guard.arm_block_reason" in text` and a mutant that reverted
        # scripts/batch-merge.py's predicate to the old author-AND-title conjunction
        # SURVIVED — the phrase still appeared, in the function's DOCSTRING. A prose
        # mention is not a call site.
        import ast

        for filename, symbol in sorted(self.PATHS.items()):
            with self.subTest(script=filename):
                tree = ast.parse((SCRIPTS / filename).read_text(encoding="utf-8"))
                calls = [
                    node
                    for node in ast.walk(tree)
                    if isinstance(node, ast.Call)
                    and isinstance(node.func, ast.Attribute)
                    and node.func.attr == symbol
                    and isinstance(node.func.value, ast.Name)
                    and node.func.value.id == "release_pr_guard"
                ]
                self.assertTrue(
                    calls,
                    f"scripts/{filename} contains no CALL to "
                    f"release_pr_guard.{symbol} — an arming/merging path lost its "
                    "#1135 Release-PR exclusion. (A comment, docstring or import "
                    "mentioning the guard does not exclude anything.)",
                )
    def test_batch_merge_and_pr_backlog_catch_a_branch_only_release_pr(self) -> None:
        """BEHAVIOURAL, not structural: the case the OLD conjunction missed.

        `author == github-actions AND title startswith "chore: release"` returns False for
        a Release PR whose title was edited or whose author identity changed, even though
        its head branch is unmistakable. Both scripts must now catch it on the branch
        alone. This is the assertion that killed the surviving mutant.
        """
        batch = _load("batch_merge_under_test", "batch-merge.py")
        backlog = _load("pr_backlog_under_test", "pr-backlog.py")
        branch_only = {
            "head_ref": "release-plz-main",
            "author_login": "app/some-other-bot",
            "title": "Bump workspace version to 0.2.0",
        }
        ordinary = {
            "head_ref": "sparq-agent/issue-3801-x",
            "author_login": "app/sparq-orchestrator",
            "title": "fix(engine): a change",
        }
        for name, module in (("batch-merge", batch), ("pr-backlog", backlog)):
            with self.subTest(script=name):
                self.assertTrue(
                    module.is_release_plz(branch_only),
                    f"{name}.is_release_plz missed a Release PR identifiable by its head "
                    "branch alone — the old author-AND-title conjunction is back.",
                )
                # The discriminating half: it must not swallow ordinary worker PRs.
                self.assertFalse(module.is_release_plz(ordinary), name)

    def test_the_predicate_cannot_be_influenced_by_labels(self) -> None:
        # The whole point of #1135's keying decision: a label can be added by anything
        # holding pull-requests: write, so a label-keyed exclusion is defeated by
        # relabelling. Structural, so it survives a rewrite of the body.
        import inspect

        params = inspect.signature(release_pr_guard.arm_block_reason).parameters
        self.assertNotIn("labels", params)
        self.assertEqual(
            sorted(params), ["author_login", "head_ref", "title"], sorted(params)
        )

    def test_release_pr_is_blocked_and_a_worker_pr_is_not(self) -> None:
        # The discriminating pair. If either half stops holding the guard is either
        # useless (blocks nothing) or a merge-train brick (blocks everything).
        self.assertIsNotNone(
            release_pr_guard.arm_block_reason(
                head_ref="release-plz-main",
                author_login="app/github-actions",
                title="chore: release v0.2.0",
            )
        )
        self.assertIsNone(
            release_pr_guard.arm_block_reason(
                head_ref="sparq-agent/issue-3801-x",
                author_login="app/sparq-orchestrator",
                title="fix(engine): a change",
            )
        )

    def test_indeterminate_head_branch_blocks(self) -> None:
        for head in (None, "", "   "):
            with self.subTest(head=head):
                self.assertIsNotNone(
                    release_pr_guard.arm_block_reason(
                        head_ref=head, author_login="jeswr", title="fix: x"
                    )
                )


class TestReleasePrGuardEndToEndThroughTheHook(unittest.TestCase):
    """Drive scripts/check-pr-arm-base.py as the harness does: hook JSON on stdin."""

    def _decide(self, command: str, gh_stdout: str | None) -> str | None:
        with tempfile.TemporaryDirectory(prefix="sparq-1135-hook-") as tmp:
            fake_bin = Path(tmp) / "bin"
            fake_bin.mkdir()
            gh = fake_bin / "gh"
            if gh_stdout is None:
                gh.write_text("#!/bin/sh\nexit 1\n", encoding="utf-8")
            else:
                gh.write_text(
                    "#!/bin/sh\nprintf '%s\\n' " + repr(gh_stdout).replace('"', '\\"'),
                    encoding="utf-8",
                )
                gh.write_text(
                    "#!/bin/sh\ncat <<'JSON'\n" + gh_stdout + "\nJSON\n",
                    encoding="utf-8",
                )
            gh.chmod(0o755)
            env = os.environ.copy()
            env["PATH"] = f"{fake_bin}{os.pathsep}{env.get('PATH', '')}"
            env["CLAUDE_PROJECT_DIR"] = str(REPO_ROOT)
            proc = subprocess.run(
                [sys.executable, str(SCRIPTS / "check-pr-arm-base.py")],
                input=json.dumps({"tool_input": {"command": command}}),
                capture_output=True,
                text=True,
                env=env,
                timeout=30,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr)
            return json.loads(proc.stdout)["hookSpecificOutput"]["permissionDecision"]

    RELEASE_PR = json.dumps(
        {
            "baseRefName": "main",
            "headRefName": "release-plz-main",
            "number": 900,
            "author": {"login": "app/github-actions"},
            "title": "chore: release v0.2.0",
        }
    )
    WORKER_PR = json.dumps(
        {
            "baseRefName": "main",
            "headRefName": "sparq-agent/issue-3801-x",
            "number": 3801,
            "author": {"login": "app/sparq-orchestrator"},
            "title": "fix(engine): a change",
        }
    )

    def test_arming_the_release_pr_is_denied(self) -> None:
        self.assertEqual(
            self._decide("gh pr merge 900 --auto --squash", self.RELEASE_PR), "deny"
        )

    def test_arming_an_ordinary_worker_pr_is_allowed(self) -> None:
        # Same command shape, same base (`main`), same everything except the PR's
        # identity — so the deny above is attributable to the guard and nothing else.
        self.assertEqual(
            self._decide("gh pr merge 3801 --auto --squash", self.WORKER_PR), "allow"
        )

    def test_an_unresolvable_pr_is_denied_fail_closed(self) -> None:
        self.assertEqual(
            self._decide("gh pr merge 3801 --auto --squash", None), "deny"
        )


class TestSettingsJsonWrapperDoesNotInvertTheGuard(unittest.TestCase):
    """The hook WRAPPER in .claude/settings.json must not invert the script (#4192, f3).

    `scripts/check-pr-arm-base.py` deliberately fails CLOSED on the release axis. Its
    wiring used to undo that: the shell wrapper was

        if [ -r "$guard" ] && guard_output=$(python3 "$guard"); then …
        else <permissionDecision: allow>

    so whenever the guard could not RUN at all — deleted, unreadable, crashing, or
    `CLAUDE_PROJECT_DIR` unset — the wrapper emitted **allow**. Measured on PR #4192's
    head: all three of those cases returned `allow` for `gh pr merge 900 --auto --squash`.
    An agent in a checkout where the guard is missing could therefore arm the Release PR,
    which is the one thing this hook exists to prevent.

    These tests execute the wrapper string out of settings.json as a real subprocess.
    Reverting the `elif` back to a blanket `allow` reds
    `test_an_unrunnable_guard_DENIES_an_arm`.
    """

    SETTINGS = REPO_ROOT / ".claude" / "settings.json"

    @classmethod
    def setUpClass(cls) -> None:
        settings = json.loads(cls.SETTINGS.read_text(encoding="utf-8"))
        commands = [
            hook["command"]
            for entry in settings.get("hooks", {}).get("PreToolUse", [])
            for hook in entry.get("hooks", [])
            if hook.get("type") == "command"
            and "check-pr-arm-base.py" in str(hook.get("command", ""))
        ]
        assert len(commands) == 1, (
            "expected exactly one PreToolUse command hook wrapping "
            f"scripts/check-pr-arm-base.py in {cls.SETTINGS}, found {len(commands)} — "
            "the arm guard was unwired from the harness"
        )
        cls.wrapper = commands[0]

    def _decision(self, command: str, project_dir: str | None) -> str:
        env = os.environ.copy()
        if project_dir is None:
            env.pop("CLAUDE_PROJECT_DIR", None)
        else:
            env["CLAUDE_PROJECT_DIR"] = project_dir
        proc = subprocess.run(
            ["bash", "-c", self.wrapper],
            input=json.dumps({"tool_name": "Bash", "tool_input": {"command": command}}),
            capture_output=True,
            text=True,
            env=env,
            timeout=60,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        payload = json.loads(proc.stdout)
        return payload["hookSpecificOutput"]["permissionDecision"]

    def test_an_unrunnable_guard_DENIES_an_arm(self) -> None:
        arm_commands = (
            "gh pr merge 900 --auto --squash",
            "gh pr merge 900 --squash",  # the MORE dangerous, un-armed direct merge
            "bash -c 'gh pr merge 900 --auto'",
        )
        with tempfile.TemporaryDirectory(prefix="sparq-1135-wrapper-") as tmp:
            (Path(tmp) / "scripts").mkdir()
            for command in arm_commands:
                with self.subTest(case="guard absent", command=command):
                    self.assertEqual(self._decision(command, tmp), "deny")
                with self.subTest(case="CLAUDE_PROJECT_DIR unset", command=command):
                    self.assertEqual(self._decision(command, None), "deny")

            # Guard present but non-functional: exits non-zero without emitting JSON.
            crashing = Path(tmp) / "scripts" / "check-pr-arm-base.py"
            crashing.write_text("import sys\nsys.exit(3)\n", encoding="utf-8")
            for command in arm_commands:
                with self.subTest(case="guard crashes", command=command):
                    self.assertEqual(self._decision(command, tmp), "deny")

            # Guard runs but emits nothing: an empty decision is not a decision.
            crashing.write_text("print('')\n", encoding="utf-8")
            with self.subTest(case="guard emits nothing"):
                self.assertEqual(
                    self._decision("gh pr merge 900 --auto", tmp), "deny"
                )

    def test_an_unrunnable_guard_still_ALLOWS_a_non_merge_command(self) -> None:
        # The discriminating half. This hook matches EVERY Bash tool call, so a blanket
        # deny-on-failure would brick the fleet — and "deny" above would prove nothing.
        with tempfile.TemporaryDirectory(prefix="sparq-1135-wrapper-ok-") as tmp:
            (Path(tmp) / "scripts").mkdir()
            for command in ("cargo test -p sparq-core", "ls -la", "git status"):
                with self.subTest(command=command):
                    self.assertEqual(self._decision(command, tmp), "allow")

    def test_the_live_guard_is_still_delegated_to(self) -> None:
        # With the real checkout the wrapper must run the SCRIPT, not its fallback: a
        # non-merge command is allowed by the script itself, and the reason string is the
        # script's, not the wrapper's.
        self.assertEqual(self._decision("echo hello", str(REPO_ROOT)), "allow")


# ================================================================ THE CADENCE GUARD
class TestMinimumIntervalIsEnforced(unittest.TestCase):
    NOW = dt.datetime(2026, 7, 26, 12, 0, tzinfo=dt.timezone.utc)

    def _decide(self, hours_ago: float, *, version: str = "0.2.0"):
        return interval_guard.decide(
            now=self.NOW,
            workspace_version=version,
            tags=[("v0.1.0", self.NOW - dt.timedelta(hours=hours_ago))],
            crates_io_at=None,
        )

    def test_named_constant_is_24h(self) -> None:
        self.assertEqual(
            interval_guard.MIN_RELEASE_INTERVAL, dt.timedelta(hours=24)
        )

    def test_publish_inside_the_interval_is_refused(self) -> None:
        for hours in (0.0, 1.0, 12.0, 23.9):
            with self.subTest(hours=hours):
                self.assertFalse(self._decide(hours).allowed)

    def test_publish_outside_the_interval_is_permitted(self) -> None:
        # The discriminating half: if this failed too, "refused" above would prove
        # nothing (the guard could simply refuse everything).
        for hours in (24.0, 25.0, 240.0):
            with self.subTest(hours=hours):
                self.assertTrue(self._decide(hours).allowed)

    def test_crates_io_can_only_TIGHTEN_the_verdict(self) -> None:
        # A stale tag plus a recent crates.io publish must refuse: the guard takes the
        # MAX of the two sources, so neither can be used to argue for a shorter wait.
        verdict = interval_guard.decide(
            now=self.NOW,
            workspace_version="0.2.0",
            tags=[("v0.1.0", self.NOW - dt.timedelta(days=30))],
            crates_io_at=self.NOW - dt.timedelta(hours=1),
        )
        self.assertFalse(verdict.allowed)
        self.assertEqual(verdict.source, "crates.io")


class TestUnknownsRefuseRatherThanPublish(unittest.TestCase):
    NOW = dt.datetime(2026, 7, 26, 12, 0, tzinfo=dt.timezone.utc)

    def test_shallow_checkout_refuses(self) -> None:
        def shallow(_root, args):
            return "true\n" if args[:2] == ["rev-parse", "--is-shallow-repository"] else ""

        with self.assertRaises(interval_guard.GuardRefusal) as ctx:
            interval_guard.git_release_tags(REPO_ROOT, run_git=shallow)
        self.assertIn("shallow", str(ctx.exception))

    def test_unreadable_tag_list_refuses(self) -> None:
        def broken(_root, _args):
            raise interval_guard.GuardRefusal("git for-each-ref failed: boom")

        with self.assertRaises(interval_guard.GuardRefusal):
            interval_guard.git_release_tags(REPO_ROOT, run_git=broken)

    def test_unparseable_tag_date_refuses(self) -> None:
        def bad_date(_root, args):
            if args[:2] == ["rev-parse", "--is-shallow-repository"]:
                return "false\n"
            return "v0.1.0\tnot-a-date\n"

        with self.assertRaises(interval_guard.GuardRefusal):
            interval_guard.git_release_tags(REPO_ROOT, run_git=bad_date)

    def test_unreachable_crates_io_refuses(self) -> None:
        with self.assertRaises(interval_guard.GuardRefusal):
            interval_guard.crates_io_last_publish(
                ["sparq-core"],
                fetch=lambda _u: (None, "HTTP 503"),
                retry_sleep=lambda _seconds: None,
            )

    def test_transient_crates_io_failure_is_retried_but_remains_fail_closed(self) -> None:
        responses = iter(
            [(None, "request failed: TLS EOF"), (None, "HTTP 503"), (None, None)]
        )
        sleeps: list[float] = []
        self.assertIsNone(
            interval_guard.crates_io_last_publish(
                ["sparq-core"],
                fetch=lambda _url: next(responses),
                retry_sleep=sleeps.append,
            )
        )
        self.assertEqual(sleeps, list(interval_guard.CRATES_IO_RETRY_DELAYS))

        calls = 0

        def always_fails(_url):
            nonlocal calls
            calls += 1
            return None, "HTTP 503"

        with self.assertRaises(interval_guard.GuardRefusal) as ctx:
            interval_guard.crates_io_last_publish(
                ["sparq-core"],
                fetch=always_fails,
                retry_sleep=lambda _seconds: None,
            )
        self.assertEqual(calls, 3)
        self.assertIn("after 3 attempt(s)", str(ctx.exception))

    def test_a_definitive_404_is_NOT_an_unknown(self) -> None:
        # The discriminating counterpart: a successful "this crate does not exist" must
        # NOT be conflated with "I could not ask", or the first release could never ship.
        self.assertIsNone(
            interval_guard.crates_io_last_publish(
                ["sparq-core"], fetch=lambda _u: (None, None)
            )
        )

    def test_future_dated_last_release_refuses(self) -> None:
        verdict = interval_guard.decide(
            now=self.NOW,
            workspace_version="0.2.0",
            tags=[("v0.1.0", self.NOW + dt.timedelta(hours=1))],
            crates_io_at=None,
        )
        self.assertFalse(verdict.allowed)
        self.assertIn("FUTURE", verdict.reason)

    def test_publish_flag_is_read_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory(prefix="sparq-1135-flag-") as tmp:
            root = Path(tmp)
            for body, expected in (
                ("[workspace]\npublish = false\n", False),
                ("[workspace]\npublish = true\n", True),
                ("[workspace]\n", True),  # missing key -> strict
                ("", True),  # missing table -> strict
                ('[workspace]\npublish = "maybe"\n', True),  # non-boolean -> strict
            ):
                with self.subTest(body=body):
                    (root / "release-plz.toml").write_text(body, encoding="utf-8")
                    self.assertIs(
                        interval_guard.crates_io_publish_enabled(root), expected
                    )


def _make_test_repo(
    tmp: Path,
    *,
    tag_at: dt.datetime | None,
    extra_tag: tuple[str, dt.datetime] | None = None,
) -> Path:
    """A throwaway workspace at version 0.2.0, optionally tagged `v0.1.0` at `tag_at`.

    `extra_tag` adds one more annotated tag — used by the tag-push tests to create the
    `v0.2.0` tag that IS the release being cut, alongside the earlier `v0.1.0`.
    """
    root = tmp / "repo"
    (root / "crates" / "a").mkdir(parents=True)
    (root / "crates" / "b").mkdir(parents=True)
    (root / "Cargo.toml").write_text(
        '[workspace]\nmembers = ["crates/a", "crates/b"]\n'
        '[workspace.package]\nversion = "0.2.0"\n',
        encoding="utf-8",
    )
    (root / "crates" / "a" / "Cargo.toml").write_text(
        '[package]\nname = "a"\nversion.workspace = true\n', encoding="utf-8"
    )
    (root / "crates" / "b" / "Cargo.toml").write_text(
        '[package]\nname = "b"\nversion.workspace = true\n'
        '[dependencies]\na = { path = "../a", version = "0.2.0" }\n',
        encoding="utf-8",
    )
    (root / "release-plz.toml").write_text(
        "[workspace]\npublish = true\n"
        '[[package]]\nname = "a"\nversion_group = "sparq"\n'
        '[[package]]\nname = "b"\nversion_group = "sparq"\n',
        encoding="utf-8",
    )
    env = {
        **os.environ,
        "GIT_AUTHOR_NAME": "t",
        "GIT_AUTHOR_EMAIL": "t@e",
        "GIT_COMMITTER_NAME": "t",
        "GIT_COMMITTER_EMAIL": "t@e",
    }
    # [GPT-5.6] Keep the fixture hermetic when a maintainer signs commits/tags globally.
    git = lambda *a: subprocess.run(  # noqa: E731
        [
            "git",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
            "-C",
            str(root),
            *a,
        ],
        check=True,
        capture_output=True,
        env=env,
    )
    git("init", "-q", "-b", "main")
    git("add", "-A")
    git("commit", "-qm", "init")
    for name, when in (("v0.1.0", tag_at), extra_tag or (None, None)):
        if name is None or when is None:
            continue
        env["GIT_COMMITTER_DATE"] = when.isoformat()
        subprocess.run(
            [
                "git",
                "-c",
                "tag.gpgsign=false",
                "-C",
                str(root),
                "tag",
                "-a",
                name,
                "-m",
                "r",
            ],
            check=True,
            capture_output=True,
            env=env,
        )
    return root



class TestGuardOnARealGitRepository(unittest.TestCase):
    """End-to-end over a REAL git repo + real manifests, with only crates.io injected.

    The unit tests above stub `run_git`. This one does not: it proves the shallow check,
    the tag parse, and the version-group check work against actual git output — the
    integration seam a stubbed test cannot reach.
    """

    def test_recent_real_tag_refuses_and_an_old_one_permits(self) -> None:
        now = dt.datetime.now(dt.timezone.utc)
        never_published = lambda _url: (None, None)  # noqa: E731
        with tempfile.TemporaryDirectory(prefix="sparq-1135-git-") as tmp:
            recent = _make_test_repo(Path(tmp), tag_at=now - dt.timedelta(hours=2))
            log: list[str] = []
            code = interval_guard.run(
                recent, dry_run=False, now=now, fetch=never_published, log=log.append
            )
            self.assertEqual(code, 1, "\n".join(log))
            self.assertTrue(any("REFUSED" in line for line in log), log)

        with tempfile.TemporaryDirectory(prefix="sparq-1135-git-ok-") as tmp:
            old = _make_test_repo(Path(tmp), tag_at=now - dt.timedelta(days=5))
            log = []
            code = interval_guard.run(
                old, dry_run=False, now=now, fetch=never_published, log=log.append
            )
            self.assertEqual(code, 0, "\n".join(log))
            self.assertTrue(any("ALLOW" in line for line in log), log)

    def test_a_tag_push_inside_the_window_is_REFUSED_and_the_flag_is_what_does_it(
        self,
    ) -> None:
        """The discriminating pair for the #2552 tag-push seam.

        SAME repository, SAME clock, SAME crates.io answer — the only difference is
        `released_tag`. Without it the guard sees `v0.2.0` in the tag list, concludes
        "already tagged, `release-plz release` is a no-op" and ALLOWS: exactly the vacuity
        that would make the step in release.yml decorative. With it, the tag being cut is
        excluded and the 2h-old `v0.1.0` correctly refuses.
        """
        now = dt.datetime.now(dt.timezone.utc)
        never_published = lambda _url: (None, None)  # noqa: E731
        with tempfile.TemporaryDirectory(prefix="sparq-2552-tagpush-") as tmp:
            root = _make_test_repo(
                Path(tmp),
                tag_at=now - dt.timedelta(hours=2),  # v0.1.0: the PREVIOUS release
                extra_tag=("v0.2.0", now),  # the release being cut RIGHT NOW
            )
            vacuous: list[str] = []
            self.assertEqual(
                interval_guard.run(
                    root,
                    dry_run=False,
                    now=now,
                    fetch=never_published,
                    log=vacuous.append,
                ),
                0,
                "the no-op short-circuit is expected to allow here — if it does not, the "
                "pair below no longer discriminates:\n" + "\n".join(vacuous),
            )
            self.assertTrue(any("already tagged" in line for line in vacuous), vacuous)

            guarded: list[str] = []
            code = interval_guard.run(
                root,
                dry_run=False,
                now=now,
                released_tag="v0.2.0",
                fetch=never_published,
                log=guarded.append,
            )
            self.assertEqual(
                code,
                1,
                "a `v0.2.0` tag pushed 2h after `v0.1.0` was ALLOWED. --released-tag must "
                "exclude the tag being cut and suppress the already-tagged short-circuit, "
                "or the guard on release.yml is vacuous.\n" + "\n".join(guarded),
            )
            self.assertTrue(any("REFUSED" in line for line in guarded), guarded)

    def test_a_tag_push_outside_the_window_is_ALLOWED(self) -> None:
        # The paired positive: the guard must not simply refuse every tag push.
        now = dt.datetime.now(dt.timezone.utc)
        never_published = lambda _url: (None, None)  # noqa: E731
        with tempfile.TemporaryDirectory(prefix="sparq-2552-tagpush-ok-") as tmp:
            root = _make_test_repo(
                Path(tmp),
                tag_at=now - dt.timedelta(days=5),
                extra_tag=("v0.2.0", now),
            )
            log: list[str] = []
            code = interval_guard.run(
                root,
                dry_run=False,
                now=now,
                released_tag="v0.2.0",
                fetch=never_published,
                log=log.append,
            )
            self.assertEqual(code, 0, "\n".join(log))
            self.assertTrue(any("ALLOW" in line for line in log), log)

    def test_a_tag_push_consults_crates_io_even_though_the_version_is_tagged(
        self,
    ) -> None:
        # run() skips the ~26 registry requests when the version is already tagged. On the
        # tag-push path it always IS, so that skip must not apply — otherwise a release
        # cut by hand minutes after a crates.io publish has no bound at all.
        now = dt.datetime.now(dt.timezone.utc)
        urls: list[str] = []

        def fetch_recent(url: str):
            urls.append(url)
            return (
                {"versions": [{"created_at": (now - dt.timedelta(hours=1)).isoformat()}]},
                None,
            )

        with tempfile.TemporaryDirectory(prefix="sparq-2552-cratesio-") as tmp:
            # No previous tag: crates.io is the ONLY release signal in existence.
            root = _make_test_repo(Path(tmp), tag_at=None, extra_tag=("v0.2.0", now))
            log: list[str] = []
            code = interval_guard.run(
                root,
                dry_run=False,
                now=now,
                released_tag="v0.2.0",
                fetch=fetch_recent,
                log=log.append,
            )
            self.assertTrue(
                urls,
                "run() never queried crates.io on the tag-push path. The version is "
                "always already tagged there, so reusing the no-op skip leaves an "
                "UNBOUNDED cadence.\n" + "\n".join(log),
            )
            self.assertEqual(code, 1, "\n".join(log))

    def test_a_non_release_released_tag_REFUSES(self) -> None:
        # Fail-closed: if the guard cannot tell which tag to exclude, it would measure the
        # interval against the very release it is deciding.
        now = dt.datetime.now(dt.timezone.utc)
        never_published = lambda _url: (None, None)  # noqa: E731
        with tempfile.TemporaryDirectory(prefix="sparq-2552-badtag-") as tmp:
            root = _make_test_repo(Path(tmp), tag_at=now - dt.timedelta(days=5))
            log: list[str] = []
            code = interval_guard.run(
                root,
                dry_run=False,
                now=now,
                released_tag="release-candidate",
                fetch=never_published,
                log=log.append,
            )
            self.assertEqual(code, 1, "\n".join(log))
            self.assertTrue(any("REFUSED" in line for line in log), log)

    def test_run_ACTUALLY_fetches_crates_io_when_the_version_is_untagged(self) -> None:
        """The CALL SITE, not the pure function (PR #4192 review, finding 2).

        `test_crates_io_can_only_TIGHTEN_the_verdict` passes `crates_io_at` into
        `decide()` as a parameter, so it cannot notice `run()` ceasing to fetch. Two
        realistic edits survived it — someone deleting the ~26 registry requests to make
        the release job faster or less flaky:

            run(): crates_io_at = None            (never fetch)
            run(): fetch, then discard the result

        This fixture is the MAINTAINER'S HAND-BOOTSTRAP WINDOW (docs/release.md checklist
        step 3): crates are published by hand, leaf-first, and **no tag is cut**. With no
        `v*` tag, crates.io is the ONLY release signal in existence, so a `run()` that
        skips or discards it has no interval bound at exactly the moment the interval
        matters most — it would fall through to the "FIRST release, no cadence can have
        been violated" branch and allow.
        """
        now = dt.datetime.now(dt.timezone.utc)
        urls: list[str] = []

        def fetch_recent(url: str):
            urls.append(url)
            return (
                {
                    "versions": [
                        {"created_at": (now - dt.timedelta(hours=2)).isoformat()}
                    ]
                },
                None,
            )

        with tempfile.TemporaryDirectory(prefix="sparq-1135-callsite-") as tmp:
            # tag_at=None: NO `v*` tag at all — the bootstrap window.
            root = _make_test_repo(Path(tmp), tag_at=None)
            log: list[str] = []
            code = interval_guard.run(
                root, dry_run=False, now=now, fetch=fetch_recent, log=log.append
            )
            self.assertTrue(
                urls,
                "run() never called `fetch` even though no `v0.2.0` tag exists. During "
                "the hand-bootstrap window crates.io is the only release signal, so an "
                "unfetched registry means an UNBOUNDED publish cadence.\n"
                + "\n".join(log),
            )
            self.assertEqual(
                code,
                1,
                "run() did not refuse despite crates.io reporting a publication 2h ago "
                "and the minimum interval being 24h — the fetched timestamp is being "
                "discarded before it reaches decide().\n" + "\n".join(log),
            )
            self.assertTrue(
                any("crates.io" in line for line in log),
                "the refusal does not attribute itself to crates.io\n" + "\n".join(log),
            )

    def test_run_permits_when_crates_io_reports_an_OLD_publication(self) -> None:
        # The discriminating counterpart to the test above: identical wiring, identical
        # absence of tags, only the crates.io timestamp differs. Without this, "refused"
        # above would prove nothing — the guard could simply refuse whenever it fetches.
        now = dt.datetime.now(dt.timezone.utc)
        urls: list[str] = []

        def fetch_old(url: str):
            urls.append(url)
            return (
                {"versions": [{"created_at": (now - dt.timedelta(days=40)).isoformat()}]},
                None,
            )

        with tempfile.TemporaryDirectory(prefix="sparq-1135-callsite-ok-") as tmp:
            root = _make_test_repo(Path(tmp), tag_at=None)
            log: list[str] = []
            code = interval_guard.run(
                root, dry_run=False, now=now, fetch=fetch_old, log=log.append
            )
            self.assertTrue(urls, "\n".join(log))
            self.assertEqual(code, 0, "\n".join(log))
            self.assertTrue(any("ALLOW" in line for line in log), log)

    def test_run_does_not_fetch_crates_io_on_an_already_tagged_no_op_push(self) -> None:
        # The deliberate short-circuit, pinned so it stays deliberate: when
        # `v<workspace version>` is already tagged, `release-plz release` is a no-op and
        # the ~26 registry requests are skipped. If someone later "fixes" the fetch by
        # making it unconditional, this reds and they must decide on purpose.
        now = dt.datetime.now(dt.timezone.utc)
        urls: list[str] = []

        def fetch(url: str):
            urls.append(url)
            return (None, None)

        with tempfile.TemporaryDirectory(prefix="sparq-1135-noop-") as tmp:
            root = _make_test_repo(Path(tmp), tag_at=now - dt.timedelta(days=5))
            # Retag as the CURRENT workspace version so the push is a no-op.
            subprocess.run(
                ["git", "-c", "tag.gpgsign=false", "-C", str(root), "tag", "v0.2.0"],
                check=True,
                capture_output=True,
            )
            log: list[str] = []
            code = interval_guard.run(
                root, dry_run=False, now=now, fetch=fetch, log=log.append
            )
            self.assertEqual(code, 0, "\n".join(log))
            self.assertEqual(urls, [], "\n".join(log))

    def test_version_group_drift_refuses_when_publish_is_enabled(self) -> None:
        now = dt.datetime.now(dt.timezone.utc)
        with tempfile.TemporaryDirectory(prefix="sparq-1135-drift-") as tmp:
            root = _make_test_repo(Path(tmp), tag_at=now - dt.timedelta(days=5))
            # Drop crate `b` from the version_group: it stays cargo-publishable.
            (root / "release-plz.toml").write_text(
                "[workspace]\npublish = true\n"
                '[[package]]\nname = "a"\nversion_group = "sparq"\n',
                encoding="utf-8",
            )
            log: list[str] = []
            code = interval_guard.run(
                root, dry_run=False, now=now, fetch=lambda _u: (None, None), log=log.append
            )
            self.assertEqual(code, 1, "\n".join(log))
            self.assertTrue(any("version_group" in line for line in log), log)

    def test_version_group_drift_only_WARNS_while_publish_is_false(self) -> None:
        now = dt.datetime.now(dt.timezone.utc)
        with tempfile.TemporaryDirectory(prefix="sparq-1135-drift-off-") as tmp:
            root = _make_test_repo(Path(tmp), tag_at=now - dt.timedelta(days=5))
            (root / "release-plz.toml").write_text(
                "[workspace]\npublish = false\n"
                '[[package]]\nname = "a"\nversion_group = "sparq"\n',
                encoding="utf-8",
            )
            log: list[str] = []
            code = interval_guard.run(
                root, dry_run=False, now=now, fetch=lambda _u: (None, None), log=log.append
            )
            self.assertEqual(code, 0, "\n".join(log))
            self.assertTrue(any("::warning" in line for line in log), log)


class TestPublishableDependencyClosure(unittest.TestCase):
    """A registry package cannot retain workspace-only dependency edges."""

    def test_release_runbook_order_matches_the_live_dependency_graph(self) -> None:
        crates = interval_guard.publishable_crates(REPO_ROOT)
        expected = [crate.name for crate in interval_guard.publish_order(crates)]
        documented = re.findall(
            r"^cargo publish -p ([A-Za-z0-9_-]+)$",
            (REPO_ROOT / "docs" / "release.md").read_text(encoding="utf-8"),
            flags=re.MULTILINE,
        )
        self.assertEqual(documented, expected)

    @staticmethod
    def _fixture(
        root: Path, *, dependency_publishable: bool, dependency_version: str | None
    ) -> None:
        (root / "crates" / "a").mkdir(parents=True)
        (root / "crates" / "b").mkdir(parents=True)
        (root / "Cargo.toml").write_text(
            '[workspace]\nmembers = ["crates/a", "crates/b"]\n'
            '[workspace.package]\nversion = "0.2.0"\n',
            encoding="utf-8",
        )
        version = (
            f', version = "{dependency_version}"' if dependency_version is not None else ""
        )
        (root / "crates" / "a" / "Cargo.toml").write_text(
            '[package]\nname = "a"\nversion.workspace = true\n'
            f'[dependencies]\nb = {{ path = "../b"{version} }}\n',
            encoding="utf-8",
        )
        private = "" if dependency_publishable else "publish = false\n"
        (root / "crates" / "b" / "Cargo.toml").write_text(
            '[package]\nname = "b"\nversion.workspace = true\n' + private,
            encoding="utf-8",
        )

    def test_public_to_private_workspace_dependency_refuses(self) -> None:
        with tempfile.TemporaryDirectory(prefix="sparq-publish-private-dep-") as tmp:
            root = Path(tmp)
            self._fixture(
                root, dependency_publishable=False, dependency_version="0.2.0"
            )
            with self.assertRaisesRegex(
                interval_guard.GuardRefusal, "unpublished workspace crate"
            ):
                interval_guard.publishable_crates(root)

    def test_path_dependency_without_registry_version_refuses(self) -> None:
        with tempfile.TemporaryDirectory(prefix="sparq-publish-unversioned-dep-") as tmp:
            root = Path(tmp)
            self._fixture(root, dependency_publishable=True, dependency_version=None)
            with self.assertRaisesRegex(
                interval_guard.GuardRefusal, "no registry version requirement"
            ):
                interval_guard.publishable_crates(root)

    def test_versioned_publishable_dependency_enters_publish_order(self) -> None:
        with tempfile.TemporaryDirectory(prefix="sparq-publish-closed-deps-") as tmp:
            root = Path(tmp)
            self._fixture(root, dependency_publishable=True, dependency_version="0.2.0")
            crates = interval_guard.publishable_crates(root)
            self.assertEqual(
                [crate.name for crate in interval_guard.publish_order(crates)],
                ["b", "a"],
            )


class TestDryRunIsInert(unittest.TestCase):
    """`--dry-run` reports and mutates NOTHING."""

    def _repo(self, tmp: Path) -> Path:
        return _make_test_repo(tmp, tag_at=dt.datetime.now(dt.timezone.utc))

    def test_dry_run_leaves_the_repository_byte_identical(self) -> None:
        with tempfile.TemporaryDirectory(prefix="sparq-1135-dry-") as tmp:
            root = self._repo(Path(tmp))

            def snapshot() -> tuple:
                show = lambda *a: subprocess.run(  # noqa: E731
                    ["git", "-C", str(root), *a], capture_output=True, text=True
                ).stdout
                files = sorted(
                    (p.relative_to(root).as_posix(), p.read_bytes())
                    for p in root.rglob("*")
                    if p.is_file() and ".git/" not in p.as_posix()
                )
                return (show("rev-parse", "HEAD"), show("tag", "-l"), show("status", "--porcelain"), files)

            before = snapshot()
            log: list[str] = []
            code = interval_guard.run(
                root,
                dry_run=True,
                now=dt.datetime.now(dt.timezone.utc),
                fetch=lambda _u: (None, None),
                log=log.append,
            )
            self.assertEqual(code, 0, "\n".join(log))
            self.assertEqual(before, snapshot(), "--dry-run mutated the repository")

    def test_dry_run_never_invokes_a_publishing_command(self) -> None:
        # Record every git invocation and assert none of them writes. `cargo publish` is
        # unreachable by construction (the module never spawns cargo), asserted below.
        seen: list[list[str]] = []

        def recording_git(root, args):
            seen.append(list(args))
            return interval_guard._run_git(root, args)

        with tempfile.TemporaryDirectory(prefix="sparq-1135-dry2-") as tmp:
            root = self._repo(Path(tmp))
            interval_guard.run(
                root,
                dry_run=True,
                now=dt.datetime.now(dt.timezone.utc),
                fetch=lambda _u: (None, None),
                git_runner=recording_git,
                log=lambda _m: None,
            )
        self.assertTrue(seen, "the dry run issued no git command at all")
        forbidden = {"tag", "push", "commit", "checkout", "reset", "clean", "update-ref"}
        for args in seen:
            self.assertNotIn(
                args[0], forbidden, f"--dry-run issued a mutating git command: {args}"
            )
    def test_the_guard_can_only_ever_spawn_git(self) -> None:
        # STRUCTURAL, not textual (the module's docstring legitimately says "cargo
        # publish"): every subprocess.run in the guard must have `git` as argv[0]. This is
        # what makes "--dry-run touches nothing" a property of the module rather than of
        # the one code path the test above happened to walk.
        import ast

        tree = ast.parse(
            (SCRIPTS / "release-interval-guard.py").read_text(encoding="utf-8")
        )
        spawns = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Call)
            and isinstance(node.func, ast.Attribute)
            and node.func.attr in {"run", "Popen", "call", "check_output", "check_call"}
            and isinstance(node.func.value, ast.Name)
            and node.func.value.id == "subprocess"
        ]
        self.assertTrue(spawns, "no subprocess call found — re-point this test")
        for call in spawns:
            argv = call.args[0] if call.args else None
            self.assertIsInstance(
                argv, ast.List, f"line {call.lineno}: argv is not a literal list"
            )
            first = argv.elts[0]
            self.assertIsInstance(first, ast.Constant, f"line {call.lineno}")
            self.assertEqual(
                first.value,
                "git",
                f"line {call.lineno}: the cadence guard spawns {first.value!r}. It must "
                "only ever run `git` — spawning cargo (or anything else) would make it "
                "capable of the very publish it exists to prevent.",
            )

    def test_dry_run_reports_crates_versions_and_order(self) -> None:
        with tempfile.TemporaryDirectory(prefix="sparq-1135-dry3-") as tmp:
            root = self._repo(Path(tmp))
            log: list[str] = []
            interval_guard.run(
                root,
                dry_run=True,
                now=dt.datetime.now(dt.timezone.utc),
                fetch=lambda _u: (None, None),
                log=log.append,
            )
        text = "\n".join(log)
        self.assertIn("DRY-RUN", text)
        self.assertIn("publish order", text)
        # Dependency-first: `a` before `b`, because b depends on a.
        self.assertLess(text.index("1. a"), text.index("2. b"), text)
        self.assertIn("0.2.0", text)

    def test_dry_run_exits_zero_even_when_it_would_refuse(self) -> None:
        with tempfile.TemporaryDirectory(prefix="sparq-1135-dry4-") as tmp:
            root = self._repo(Path(tmp))  # tagged NOW -> inside the interval
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["crates/a", "crates/b"]\n'
                '[workspace.package]\nversion = "0.3.0"\n',  # untagged -> would release
                encoding="utf-8",
            )
            log: list[str] = []
            code = interval_guard.run(
                root,
                dry_run=True,
                now=dt.datetime.now(dt.timezone.utc),
                fetch=lambda _u: (None, None),
                log=log.append,
            )
            self.assertEqual(code, 0)
            self.assertTrue(any("REFUSE" in line for line in log), log)


class TestLivePublishFlagIsStillFalse(unittest.TestCase):
    """A tripwire, not a policy: flipping `publish = true` is a MAINTAINER decision and
    must be a deliberate, reviewed diff — not something that rides in on an unrelated PR.
    When the maintainer flips it, this test is updated in the SAME commit."""

    def test_release_plz_toml_still_has_publish_false(self) -> None:
        self.assertFalse(
            interval_guard.crates_io_publish_enabled(REPO_ROOT),
            "release-plz.toml now enables crates.io publishing. If that is intended, the "
            "maintainer checklist in the #1135 PR body must be complete (version_group "
            "reconciled, Trusted Publishing configured) and this test updated in the "
            "same commit.",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
