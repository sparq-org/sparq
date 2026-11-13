"""Exercise the distribution notice program with controlled dependency metadata."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "license-notices.py"


class NoticeProgram(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.package = self.root / "dependency"
        self.package.mkdir()
        self.output = self.root / "notices"
        self.metadata = {
            "packages": [{"id": "root"}, {"id": "dependency", "name": "fixture-dependency", "version": "1.0.0", "license": "MIT", "manifest_path": str(self.package / "Cargo.toml")}],
            "resolve": {"root": "root", "nodes": [{"id": "root", "deps": [{"pkg": "dependency"}]}, {"id": "dependency", "deps": []}]},
        }
        # Only the metadata boundary is controlled; the real program reads actual files.
        cargo = self.root / "cargo"
        cargo.write_text('#!/usr/bin/env python3\nimport pathlib\nprint(pathlib.Path(__file__).with_name("metadata.json").read_text())\n')
        cargo.chmod(0o755)
        self.env = dict(os.environ, PATH=str(self.root) + os.pathsep + os.environ["PATH"])

    def run_program(self):
        (self.root / "metadata.json").write_text(json.dumps(self.metadata))
        return subprocess.run(["python3", str(SCRIPT), str(self.root / "Cargo.toml"), str(self.output)], env=self.env, capture_output=True, text=True, check=False)

    def test_copies_reachable_notices_and_copyright(self):
        (self.package / "LICENSE-MIT").write_text("fixture permission text")
        (self.package / "COPYRIGHT").write_text("fixture copyright text")
        self.metadata["packages"].append({"id": "unreachable"})
        self.assertEqual(self.run_program().returncode, 0)
        text = self.output.read_text()
        for expected in ["fixture-dependency", "SPDX: MIT", "fixture permission text", "fixture copyright text"]:
            self.assertIn(expected, text)
        self.assertNotIn("unreachable", text)

    def test_accepts_explicit_licence_file_without_spdx(self):
        (self.package / "terms.txt").write_text("explicit fixture terms")
        self.metadata["packages"][1].update(license=None, license_file="terms.txt")
        self.assertEqual(self.run_program().returncode, 0)
        self.assertIn("explicit fixture terms", self.output.read_text())

    def test_missing_notice_fails_without_output(self):
        result = self.run_program()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing licence notices: fixture-dependency", result.stderr)
        self.assertFalse(self.output.exists())

    def test_licence_file_cannot_escape_the_dependency(self):
        (self.root / "outside.txt").write_text("outside fixture contents")
        self.metadata["packages"][1]["license_file"] = "../outside.txt"
        result = self.run_program()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("licence path outside package", result.stderr)
        self.assertFalse(self.output.exists())

    def test_missing_selected_package_fails(self):
        (self.package / "LICENSE").write_text("fixture licence text")
        self.metadata["resolve"]["root"] = None
        result = self.run_program()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("dependency metadata has no selected package", result.stderr)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
