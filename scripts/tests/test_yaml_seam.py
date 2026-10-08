"""The PyYAML skip in _yaml_seam.py is LOCAL-ONLY (#5820).

A stub `yaml` module that raises ImportError stands in for a missing PyYAML.
"""
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
PROBE = (
    "import sys; sys.path.insert(0, {here!r})\n"
    "from _yaml_seam import yaml_or_local_skip\n"
    "yaml = yaml_or_local_skip()\n"
    "print('LOADED')\n"
)


def _run(extra_env: dict) -> subprocess.CompletedProcess:
    with tempfile.TemporaryDirectory() as d:
        (pathlib.Path(d) / "yaml.py").write_text("raise ImportError('simulated: no yaml')\n")
        env = {k: v for k, v in os.environ.items() if k not in ("CI", "GITHUB_ACTIONS")}
        env.update(extra_env, PYTHONPATH=d)
        return subprocess.run(
            [sys.executable, "-c", PROBE.format(here=str(HERE))],
            env=env, capture_output=True, text=True,
        )


class YamlSeamIsLocalOnly(unittest.TestCase):
    def test_local_run_skips_loudly(self):
        r = _run({})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("SKIPPED", r.stderr)
        self.assertNotIn("LOADED", r.stdout)

    def test_ci_run_hard_fails(self):
        for var in ("CI", "GITHUB_ACTIONS"):
            r = _run({var: "true"})
            self.assertNotEqual(r.returncode, 0, var)
            self.assertIn("ImportError", r.stderr)


if __name__ == "__main__":
    unittest.main()
