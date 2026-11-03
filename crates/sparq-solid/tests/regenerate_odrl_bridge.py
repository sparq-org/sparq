#!/usr/bin/env python3
"""Capture full N3 bridge outcomes/graphs from pinned main.

Usage: regenerate_odrl_bridge.py OUTPUT [--check]
Generation archives the pinned main revision and installs the current observer.
--check archives the committed HEAD, regenerates its observations, and compares
those with its committed fixture. Neither mode edits the baseline checkout.
The cargo subprocess inherits the environment and repository toolchain pin.
"""
import difflib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[3]
SOURCE = "1c600eb34e5721950472915115533f56eb84041e"
OBSERVER = '''    #[cfg(test)]
    let before = bridge_golden::graph_snapshot(graph);
    let result = materialize_odrl_n3_with_budget(graph, policy_ttl, request, crate::default_auth_budget());
    #[cfg(test)]
    bridge_golden::check(policy_ttl, request, before, graph, &result);
    result'''


def main():
    output = Path(sys.argv[1]).resolve()
    output.mkdir(parents=True, exist_ok=True)
    check = "--check" in sys.argv[2:]
    revision = "HEAD" if check else SOURCE
    with tempfile.TemporaryDirectory(prefix="odrl-bridge-") as scratch:
        scratch = Path(scratch)
        archive = scratch / "source.tar"
        subprocess.run(["git", "archive", revision, "-o", str(archive)], cwd=ROOT, check=True)
        source = scratch / "source"
        source.mkdir()
        with tarfile.open(archive) as tree:
            tree.extractall(source, filter="data")
        tests = source / "crates/sparq-solid/tests"
        if not check:
            for name in ["support/odrl_bridge_golden.rs", "fixtures/odrl-bridge-main.json"]:
                (tests / name).parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / "crates/sparq-solid/tests" / name, tests / name)
        expected = (tests / "fixtures/odrl-bridge-main.json").read_text()
        target = source / "crates/sparq-solid/src/odrl_bridge.rs"
        code = target.read_text()
        seam = '    materialize_odrl_n3_with_budget(graph, policy_ttl, request, crate::default_auth_budget())'
        if "let before = bridge_golden::graph_snapshot(graph);" not in code:
            assert code.count(seam) == 1
            code = code.replace(seam, OBSERVER)
            code += '\n#[cfg(test)]\n#[path = "../tests/support/odrl_bridge_golden.rs"]\nmod bridge_golden;\n'
        target.write_text(code)
        captures = scratch / "captures"
        captures.mkdir()
        env = dict(os.environ, ODRL_BRIDGE_GOLDEN_DIR=str(captures), RUST_TEST_THREADS="2")
        subprocess.run(["cargo", "test", "-p", "sparq-solid", "--all-features", "--lib", "bridge_golden"], cwd=source, env=env, check=True)
        entries = [json.loads(path.read_text()) for path in sorted(captures.glob("*.json"))]
        entries.sort(key=lambda entry: json.dumps(entry["input"], sort_keys=True))
        actual = json.dumps(entries, indent=2, ensure_ascii=False) + "\n"
        (output / "odrl-bridge-main.json").write_text(actual)
        (output / "source-revision.txt").write_text(subprocess.check_output(["git", "rev-parse", revision], cwd=ROOT, text=True))
        if check and actual != expected:
            difference = difflib.unified_diff(expected.splitlines(True), actual.splitlines(True), fromfile="committed-fixture", tofile="committed-head-observations")
            (output / "golden.diff").write_text("".join(difference))
            raise SystemExit(f"committed-head goldens differ: {output / 'golden.diff'}")


if __name__ == "__main__":
    main()
