"""wasm-deps-guard.sh must fail LOUD when `cargo tree` fails (#6093).

A stub `cargo` on PATH stands in for the real one so the test is hermetic.
"""
import os
import pathlib
import stat
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
GUARD = ROOT / "scripts" / "wasm-deps-guard.sh"


def _run_with_stub_cargo(body: str) -> subprocess.CompletedProcess:
    with tempfile.TemporaryDirectory() as d:
        stub = pathlib.Path(d) / "cargo"
        stub.write_text("#!/bin/sh\n" + body)
        stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
        env = dict(os.environ, PATH=f"{d}{os.pathsep}{os.environ.get('PATH', '')}")
        return subprocess.run(
            ["bash", str(GUARD)], cwd=ROOT, env=env, capture_output=True, text=True
        )


class WasmDepsGuardLoud(unittest.TestCase):
    def test_cargo_tree_failure_is_reported(self):
        r = _run_with_stub_cargo('echo "error: failed to load manifest" >&2\nexit 101\n')
        self.assertEqual(r.returncode, 1)
        self.assertIn("::error::cargo tree failed for sparq-wasm", r.stdout)
        self.assertIn("failed to load manifest", r.stdout)

    def test_clean_graph_passes(self):
        r = _run_with_stub_cargo('echo "sparq-wasm v0.1.0"\necho "└── serde v1.0.0"\n')
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("wasm-deps-guard: OK", r.stdout)

    def test_forbidden_crate_still_flagged(self):
        r = _run_with_stub_cargo('echo "sparq-wasm v0.1.0"\necho "└── rayon v1.10.0"\n')
        self.assertEqual(r.returncode, 1)
        self.assertIn("::error::forbidden crate 'rayon'", r.stdout)


if __name__ == "__main__":
    unittest.main()
