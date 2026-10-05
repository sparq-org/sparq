#!/usr/bin/env python3
"""[GPT-6] Execute the real GUI staging step against hermetic installer fixtures.

Uses /bin/bash (3.2 on macOS), overridable with RELEASE_TEST_BASH. No builds,
network, GitHub credentials, or publication calls are needed.
"""

import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = yaml.safe_load((ROOT / ".github/workflows/release.yml").read_text())
STEP = next(step for step in WORKFLOW["jobs"]["gui-bundle"]["steps"]
            if step.get("name") == "Stage GUI bundles")
BASH = os.environ.get("RELEASE_TEST_BASH", "/bin/bash")


class TestGuiStaging(unittest.TestCase):
    def stage(self, label, files, *, script=None, block_output=False):
        with tempfile.TemporaryDirectory(prefix="sparq staging ") as tmp:
            root = Path(tmp)
            bundles = root / "gui/src-tauri/target/release/bundle"
            for relative, content in files.items():
                path = bundles / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(content)
            if block_output:
                (root / "gui-bundles").write_text("not a directory")
            command = (STEP["run"] if script is None else script).replace(
                "${{ matrix.label }}", label)
            self.assertNotIn("${{", command)
            result = subprocess.run(
                [BASH, "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", command],
                cwd=root, env={"PATH": os.defpath, "VERSION": "v0.1.2"},
                capture_output=True, text=True, timeout=10,
            )
            output = root / "gui-bundles"
            staged = {p.name: p.read_bytes() for p in output.iterdir()} if output.is_dir() else {}
            return result, staged

    def test_every_matrix_platform_preserves_installer_bytes_and_names(self):
        fixtures = {
            "arm64-darwin": ("aarch64", {"dmg/Sparq arm.dmg": ("dmg", b"arm dmg")}),
            "x64-darwin": ("x86_64", {"dmg/Sparq intel.dmg": ("dmg", b"intel dmg")}),
            "x64-linux": ("amd64", {
                "deb/Sparq linux.deb": ("deb", b"deb"),
                "appimage/Sparq linux.AppImage": ("AppImage", b"appimage"),
                "rpm/Sparq linux.rpm": ("rpm", b"rpm"),
            }),
            "arm64-linux": ("arm64", {
                "deb/Sparq arm.deb": ("deb", b"arm deb"),
                "appimage/Sparq arm.AppImage": ("AppImage", b"arm appimage"),
            }),
            "win-x64": ("x64", {
                "msi/Sparq windows.msi": ("msi", b"msi"),
                "nsis/Sparq setup.exe": ("-setup.exe", b"nsis"),
            }),
        }
        rows = WORKFLOW["jobs"]["gui-bundle"]["strategy"]["matrix"]["include"]
        self.assertEqual(set(fixtures), {row["label"] for row in rows})
        for label, (arch, installers) in fixtures.items():
            with self.subTest(label=label):
                files = {path: content for path, (_, content) in installers.items()}
                # App directories, metadata, and nested files are not installers.
                files.update({"macos/Sparq.app/Contents/Info.plist": b"app",
                              "dmg/nested/ignore.dmg": b"nested", "dmg/ignore.txt": b"text"})
                result, staged = self.stage(label, files)
                self.assertEqual(result.returncode, 0, result.stderr)
                expected = {
                    f"sparq-gui_v0.1.2_{arch}{suffix if suffix.startswith('-') else '.' + suffix}": content
                    for suffix, content in installers.values()
                }
                self.assertEqual(staged, expected)

    def test_missing_or_non_installer_output_fails_closed(self):
        for files in ({}, {"macos/Sparq.app/Contents/Info.plist": b"app"}):
            with self.subTest(files=files):
                result, staged = self.stage("arm64-darwin", files)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("produced no installer bundles", result.stderr)
                self.assertEqual(staged, {})

    def test_unknown_platform_fails_closed(self):
        result, staged = self.stage("unknown", {"dmg/Sparq.dmg": b"dmg"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no arch mapping", result.stderr)
        self.assertEqual(staged, {})

    def test_filesystem_failure_is_not_swallowed(self):
        result, staged = self.stage("arm64-darwin", {"dmg/Sparq.dmg": b"dmg"}, block_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(staged, {})

    def test_shell_options_remain_bash32_compatible_on_linux_ci(self):
        # Linux CI also enforces the option contract; actual macOS runs exercise Bash 3.2.
        options = [shlex.split(line) for line in STEP["run"].splitlines()
                   if line.strip().startswith("shopt ")]
        self.assertEqual(options, [["shopt", "-s", "nullglob"]])

    def test_original_globstar_failure_on_bash32(self):
        version = subprocess.check_output([BASH, "-c", "printf '%s' \"$BASH_VERSION\""], text=True)
        if not version.startswith("3.2."):
            self.skipTest("historical failure reproduction requires Bash 3.2")
        original = STEP["run"].replace("shopt -s nullglob", "shopt -s nullglob globstar")
        self.assertNotEqual(original, STEP["run"])
        result, staged = self.stage("arm64-darwin", {"dmg/Sparq.dmg": b"dmg"}, script=original)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("globstar: invalid shell option name", result.stderr)
        self.assertEqual(staged, {})

    def test_staging_and_regression_checks_cannot_be_softened(self):
        self.assertEqual(STEP["shell"], "bash")
        self.assertNotIn("if", STEP)
        self.assertNotIn("continue-on-error", STEP)
        ci = yaml.safe_load((ROOT / ".github/workflows/docs-quality.yml").read_text())
        checks = [step for job in ci["jobs"].values() for step in job.get("steps", [])
                  if "python3 scripts/tests/test_release_gui_staging.py" in step.get("run", "")]
        self.assertEqual(len(checks), 1)
        self.assertNotIn("if", checks[0])
        self.assertNotIn("continue-on-error", checks[0])


if __name__ == "__main__":
    unittest.main()
