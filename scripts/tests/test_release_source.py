#!/usr/bin/env python3
"""[GPT-6] Release source identity and fail-closed workflow wiring regressions."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/check-release-source.py"
spec = importlib.util.spec_from_file_location("release_source", SCRIPT)
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


class TestSourceIdentity(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="sparq-release-source-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.write("Cargo.toml", '[workspace]\nmembers = ["crates/sparq-py"]\n'
                   '[workspace.package]\nversion = "0.1.2"\n')
        self.write("crates/sparq-py/Cargo.toml", '[package]\nname = "sparq-py"\nversion.workspace = true\n')
        self.write("crates/sparq-py/pyproject.toml", '[project]\nname = "sparq-rdf"\ndynamic = ["version"]\n')
        for manifest in ("js/package.json", "packages/solid-server/package.json"):
            self.write(manifest, json.dumps({"version": "0.1.2"}))
        self.write("packages/eyereasoner-compat/package.json", '{"version":"0.1.0"}')
        self.write("package-lock.json", json.dumps({"packages": {
            "js": {"version": "0.1.2"},
            "packages/solid-server": {"version": "0.1.2"},
            "packages/eyereasoner-compat": {"version": "0.1.0"},
        }}))
        self.git("init", "-q")
        self.git("add", ".")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "commit.gpgsign=false", "commit", "-qm", "fixture")
        self.sha = self.git("rev-parse", "HEAD")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "tag.gpgsign=false", "tag", "-a", "v0.1.2", "-m", "fixture")

    def write(self, path, text):
        path = self.root / path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, text=True,
                                       stderr=subprocess.PIPE).strip()

    def validate(self, **overrides):
        args = dict(repo=self.root, tag="v0.1.2", ref="refs/tags/v0.1.2", commit=self.sha,
                    npm_manifests=["js/package.json", "packages/solid-server/package.json"])
        guard.validate(**(args | overrides))

    def test_matching_annotated_tag_passes(self):
        self.validate()

    def test_branch_dispatch_refuses_even_at_the_tagged_commit(self):
        with self.assertRaisesRegex(guard.SourceMismatch, "exact tag"):
            self.validate(ref="refs/heads/main")

    def test_old_tag_with_new_build_commit_refuses(self):
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "later")
        with self.assertRaisesRegex(guard.SourceMismatch, "must match"):
            self.validate(commit=self.git("rev-parse", "HEAD"))

    def test_checkout_must_match_workflow_commit(self):
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "later")
        with self.assertRaisesRegex(guard.SourceMismatch, "must match"):
            self.validate()

    def test_missing_tag_refuses(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.validate(tag="v0.1.3", ref="refs/tags/v0.1.3")

    def test_remote_annotated_tag_must_still_match(self):
        guard.validate_remote_tag(self.root, ".", "v0.1.2", self.sha)
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "later")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "tag.gpgsign=false", "tag", "-fam", "moved", "v0.1.2")
        with self.assertRaisesRegex(guard.SourceMismatch, "no longer identifies"):
            guard.validate_remote_tag(self.root, ".", "v0.1.2", self.sha)

    def test_missing_remote_tag_refuses(self):
        with self.assertRaisesRegex(guard.SourceMismatch, "does not contain"):
            guard.validate_remote_tag(self.root, ".", "v0.1.3", self.sha)

    def test_malformed_identity_refuses(self):
        for overrides in ({"tag": "v0.1.2\nversion=v9.9.9"}, {"commit": ""}):
            with self.subTest(overrides=overrides), self.assertRaises(guard.SourceMismatch):
                self.validate(**overrides)

    def test_each_manifest_version_is_checked(self):
        for manifest, content in (
            ("Cargo.toml", '[workspace]\nmembers = []\n[workspace.package]\nversion = "0.1.1"'),
            ("crates/sparq-py/Cargo.toml", '[package]\nversion = "0.1.1"'),
            ("crates/sparq-py/pyproject.toml", '[project]\nversion = "0.1.1"'),
            ("js/package.json", '{"version":"0.1.1"}'),
            ("packages/solid-server/package.json", '{"version":"0.1.1"}'),
        ):
            original = (self.root / manifest).read_text(encoding="utf-8")
            with self.subTest(manifest=manifest):
                self.write(manifest, content)
                with self.assertRaisesRegex(guard.SourceMismatch, "version does not match"):
                    self.validate()
                self.write(manifest, original)

    def test_private_implementation_crates_can_version_independently(self):
        self.write("Cargo.toml", '[workspace]\nmembers = ["crates/sparq-py", "crates/private"]\n'
                   '[workspace.package]\nversion = "0.1.2"\n')
        self.write("crates/private/Cargo.toml", '[package]\nname = "private"\n'
                   'version = "0.0.1"\npublish = false\n')
        self.validate()
        self.write("crates/private/Cargo.toml", '[package]\nname = "private"\nversion = "0.0.1"\n')
        with self.assertRaisesRegex(guard.SourceMismatch, "version does not match"):
            self.validate()

    def test_each_selected_npm_lock_workspace_version_is_checked(self):
        lock_path = self.root / "package-lock.json"
        for workspace in ("js", "packages/solid-server"):
            original = lock_path.read_text(encoding="utf-8")
            with self.subTest(workspace=workspace):
                lock = json.loads(original)
                lock["packages"][workspace]["version"] = "0.1.1"
                self.write("package-lock.json", json.dumps(lock))
                with self.assertRaisesRegex(guard.SourceMismatch, "package-lock.json workspace"):
                    self.validate()
                self.write("package-lock.json", original)

    def test_publish_checks_only_selected_npm_packages(self):
        env = os.environ | {"GITHUB_REF": "refs/tags/v0.1.2", "GITHUB_SHA": self.sha,
                            "PUBLISH_NPM": "false", "PUBLISH_SOLID_SERVER": "false",
                            "PUBLISH_EYEREASONER_COMPAT": "false"}
        command = ["python3", "-B", str(SCRIPT), "--repo-root", str(self.root),
                   "--tag", "v0.1.2", "--publish"]
        for flag, manifest in (
            ("PUBLISH_NPM", "js/package.json"),
            ("PUBLISH_SOLID_SERVER", "packages/solid-server/package.json"),
            ("PUBLISH_EYEREASONER_COMPAT", "packages/eyereasoner-compat/package.json"),
        ):
            original = (self.root / manifest).read_text(encoding="utf-8")
            with self.subTest(flag=flag):
                self.write(manifest, '{"version":"0.1.1"}')
                unselected = subprocess.run(command, env=env, capture_output=True, text=True,
                                            timeout=10)
                self.assertEqual(unselected.returncode, 0, unselected.stderr)
                selected = subprocess.run(command, env=env | {flag: "true"}, capture_output=True,
                                          text=True, timeout=10)
                self.assertEqual(selected.returncode, 1, selected.stderr)
                self.write(manifest, original)

        invalid = subprocess.run(command, env=env | {"PUBLISH_NPM": ""}, capture_output=True,
                                 text=True, timeout=10)
        self.assertEqual(invalid.returncode, 2, invalid.stderr)

    def test_publish_checks_only_selected_npm_lock_workspaces(self):
        env = os.environ | {"GITHUB_REF": "refs/tags/v0.1.2", "GITHUB_SHA": self.sha,
                            "PUBLISH_NPM": "false", "PUBLISH_SOLID_SERVER": "false",
                            "PUBLISH_EYEREASONER_COMPAT": "false"}
        command = ["python3", "-B", str(SCRIPT), "--repo-root", str(self.root),
                   "--tag", "v0.1.2", "--publish"]
        for flag, manifest, workspace in (
            ("PUBLISH_NPM", "js/package.json", "js"),
            ("PUBLISH_SOLID_SERVER", "packages/solid-server/package.json", "packages/solid-server"),
            ("PUBLISH_EYEREASONER_COMPAT", "packages/eyereasoner-compat/package.json",
             "packages/eyereasoner-compat"),
        ):
            manifest_path = self.root / manifest
            manifest_original = manifest_path.read_text(encoding="utf-8")
            lock_path = self.root / "package-lock.json"
            lock_original = lock_path.read_text(encoding="utf-8")
            with self.subTest(flag=flag):
                self.write(manifest, '{"version":"0.1.2"}')
                lock = json.loads(lock_original)
                lock["packages"][workspace]["version"] = "0.1.1"
                self.write("package-lock.json", json.dumps(lock))
                unselected = subprocess.run(command, env=env, capture_output=True, text=True,
                                            timeout=10)
                self.assertEqual(unselected.returncode, 0, unselected.stderr)
                selected = subprocess.run(command, env=env | {flag: "true"}, capture_output=True,
                                          text=True, timeout=10)
                self.assertEqual(selected.returncode, 1, selected.stderr)
                self.write(manifest, manifest_original)
                self.write("package-lock.json", lock_original)


class TestWorkflowWiring(unittest.TestCase):
    def test_build_checkouts_use_the_workflow_commit(self):
        for name in ("release", "publish", "build-matrix"):
            jobs = yaml.safe_load((ROOT / f".github/workflows/{name}.yml").read_text())["jobs"]
            for job_id, job in jobs.items():
                for step in job.get("steps", []):
                    if step.get("uses", "").startswith("actions/checkout@"):
                        with self.subTest(workflow=name, job=job_id):
                            self.assertEqual(step.get("with", {}).get("ref"), "${{ github.sha }}")

    def test_release_and_publish_are_guarded_before_work(self):
        for name in ("release", "publish"):
            with self.subTest(workflow=name):
                jobs = yaml.safe_load((ROOT / f".github/workflows/{name}.yml").read_text())["jobs"]
                setup = jobs["setup"]
                self.assertNotIn("if", setup)
                self.assertNotIn("continue-on-error", setup)
                steps = setup["steps"]
                matches = [s for s in steps if "scripts/check-release-source.py" in s.get("run", "")]
                self.assertEqual(len(matches), 1)
                check = matches[0]
                self.assertEqual(check["run"], 'python3 scripts/check-release-source.py --tag "$RELEASE_TAG"'
                                 + (" --publish" if name == "publish" else ""))
                self.assertNotIn("if", check)
                self.assertNotIn("continue-on-error", check)
                checkout = next(s for s in steps if s.get("uses", "").startswith("actions/checkout@"))
                self.assertEqual(checkout["with"]["fetch-depth"], 0)
                self.assertLess(steps.index(checkout), steps.index(check))
                if name == "release":
                    self.assertEqual(check["env"], {"RELEASE_TAG": "${{ steps.v.outputs.version }}"})
                    cadence = next(s for s in steps if "scripts/release-interval-guard.py" in s.get("run", ""))
                    self.assertLess(steps.index(check), steps.index(cadence))
                else:
                    self.assertEqual(check["env"], {
                        "RELEASE_TAG": "${{ github.event.release.tag_name || github.ref_name }}",
                        "PUBLISH_NPM": "${{ github.event_name == 'release' || inputs.publish_npm }}",
                        "PUBLISH_SOLID_SERVER": "${{ github.event_name == 'workflow_dispatch' && inputs.publish_solid_server }}",
                        "PUBLISH_EYEREASONER_COMPAT": "${{ github.event_name == 'workflow_dispatch' && inputs.publish_eyereasoner_compat }}",
                    })

                def guarded(job_id, seen=frozenset()):
                    if job_id == "setup":
                        return True
                    if job_id in seen:
                        return False
                    deps = jobs[job_id].get("needs", [])
                    if isinstance(deps, str):
                        deps = [deps]
                    return any(guarded(dep, seen | {job_id}) for dep in deps)

                for job_id in jobs:
                    self.assertTrue(guarded(job_id), f"{name}/{job_id} bypasses setup")

    def test_source_regressions_run_in_ci(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/docs-quality.yml").read_text())
        checks = [step for job in workflow["jobs"].values() for step in job.get("steps", [])
                  if "python3 scripts/tests/test_release_source.py" in step.get("run", "")]
        self.assertEqual(len(checks), 1)
        self.assertNotIn("if", checks[0])
        self.assertNotIn("continue-on-error", checks[0])

    def test_remote_tag_is_rechecked_immediately_before_publication(self):
        publish = yaml.safe_load((ROOT / ".github/workflows/publish.yml").read_text())["jobs"]
        release = yaml.safe_load((ROOT / ".github/workflows/release.yml").read_text())["jobs"]

        side_effects = (
            (publish["npm"], lambda step: "npm publish" in step.get("run", "")),
            (publish["npm-eyereasoner-compat"],
             lambda step: "npm publish" in step.get("run", "")),
            (publish["npm-solid-server"], lambda step: "npm publish" in step.get("run", "")),
            (publish["pypi-publish"],
             lambda step: step.get("uses", "").startswith("pypa/gh-action-pypi-publish@")),
            (release["release"],
             lambda step: step.get("uses", "").startswith("softprops/action-gh-release@")),
            (release["docker"],
             lambda step: step.get("uses", "").startswith("docker/build-push-action@")
             and step.get("with", {}).get("push") is True),
        )
        for job, is_side_effect in side_effects:
            steps = job["steps"]
            matches = [index for index, step in enumerate(steps) if is_side_effect(step)]
            self.assertEqual(len(matches), 1)
            index = matches[0]
            self.assertGreater(index, 0)
            guard_step = steps[index - 1]
            self.assertIn("scripts/check-release-source.py", guard_step.get("run", ""))
            self.assertIn("--remote-tag origin", guard_step["run"])
            self.assertNotIn("continue-on-error", guard_step)


class TestBootstrapMode(unittest.TestCase):
    def test_bootstrap_blocks_update_but_preserves_tag_handoff(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/release-plz.yml").read_text())
        steps = workflow["jobs"]["release-plz-pr"]["steps"]
        check = next(step for step in steps if step.get("id") == "bootstrap")
        self.assertNotIn("if", check)
        self.assertNotIn("continue-on-error", check)
        update = next(step for step in steps if step.get("id") == "releasepr")
        self.assertEqual(update["if"], "steps.bootstrap.outputs.ready == 'true'")
        self.assertLess(steps.index(check), steps.index(update))
        tag_job = workflow["jobs"]["release-plz-release"]
        self.assertNotIn("bootstrap", json.dumps(tag_job))
        self.assertNotIn("needs", tag_job)

        for git_only, publish, expected in (("true", "false", "ready=false"),
                                            ("false", "true", "ready=true"),
                                            ('"true"', "false", None)):
            with self.subTest(git_only=git_only, publish=publish), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / "release-plz.toml").write_text(
                    f"[workspace]\ngit_only={git_only}\npublish={publish}\n")
                output = root / "output"
                result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", check["run"]],
                                        cwd=root, env=os.environ | {"GITHUB_OUTPUT": str(output)},
                                        capture_output=True, text=True, timeout=10)
                if expected is None:
                    self.assertNotEqual(result.returncode, 0)
                else:
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(output.read_text().strip(), expected)


if __name__ == "__main__":
    unittest.main()
