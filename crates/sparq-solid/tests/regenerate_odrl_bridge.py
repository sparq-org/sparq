#!/usr/bin/env python3
"""Capture full N3 bridge outcomes/graphs from pinned main, never expected PR output.

Usage: regenerate_odrl_bridge.py OUTPUT [--check]
The disposable archive includes the unchanged ODRL integration corpus as unit tests
so a test-only observer sees every bridge call, including generated requests.
--check installs the same observer into an archive of the current working files.
Tracked files in the baseline checkout are never edited.
"""
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
    with tempfile.TemporaryDirectory(prefix="odrl-bridge-") as scratch:
        scratch = Path(scratch)
        archive = scratch / "source.tar"
        subprocess.run(["git", "archive", SOURCE, "-o", str(archive)], cwd=ROOT, check=True)
        source = scratch / "source"
        source.mkdir()
        with tarfile.open(archive) as tree:
            tree.extractall(source, filter="data")
        if check:
            for name in subprocess.check_output(["git", "diff", "--name-only"], cwd=ROOT, text=True).splitlines():
                if (ROOT / name).is_file():
                    shutil.copy2(ROOT / name, source / name)
        tests = source / "crates/sparq-solid/tests"
        for name in ["support/odrl_bridge_golden.rs", "fixtures/odrl-bridge-main.json"]:
            (tests / name).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / "crates/sparq-solid/tests" / name, tests / name)
        target = source / "crates/sparq-solid/src/odrl_bridge.rs"
        code = target.read_text()
        seam = '    materialize_odrl_n3_with_budget(graph, policy_ttl, request, crate::default_auth_budget())'
        if "let before = bridge_golden::graph_snapshot(graph);" not in code:
            assert code.count(seam) == 1
            code = code.replace(seam, OBSERVER)
            code += '\n#[cfg(test)]\n#[path = "../tests/support/odrl_bridge_golden.rs"]\nmod bridge_golden;\n'
        target.write_text(code)
        env = dict(os.environ, RUSTUP_TOOLCHAIN="stable", RUST_TEST_THREADS="2")
        if not check:
            env["ODRL_BRIDGE_GOLDEN_DIR"] = str(output)
        subprocess.run(["cargo", "test", "-p", "sparq-solid", "--all-features", "--lib", "bridge_golden"], cwd=source, env=env, check=True)
        if not check:
            entries = [json.loads(path.read_text()) for path in sorted(output.glob("*.json")) if path.name != "odrl-bridge-main.json"]
            entries.sort(key=lambda entry: json.dumps(entry["input"], sort_keys=True))
            (output / "odrl-bridge-main.json").write_text(json.dumps(entries, indent=2, ensure_ascii=False) + "\n")
            (output / "source-revision.txt").write_text(subprocess.check_output(["git", "rev-parse", SOURCE], cwd=ROOT, text=True))


if __name__ == "__main__":
    main()
