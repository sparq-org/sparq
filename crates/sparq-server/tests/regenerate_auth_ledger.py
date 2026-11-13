#!/usr/bin/env python3
"""Capture stateless status/body fixtures from a read-only origin/main checkout.

Usage: python3 crates/sparq-server/tests/regenerate_auth_ledger.py MAIN_CHECKOUT OUTPUT
The checkout is archived, never modified. The only injected code is the comparison
harness; expected results must never be generated from the implementation under test.
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


def main():
    baseline = Path(sys.argv[1]).resolve()
    output = Path(sys.argv[2]).resolve()
    source = subprocess.check_output(["git", "rev-parse", "origin/main"], cwd=baseline, text=True).strip()
    main_ref = subprocess.check_output(["git", "rev-parse", "origin/main"], cwd=ROOT, text=True).strip()
    if source != main_ref:
        raise RuntimeError("baseline origin/main must equal this checkout's origin/main")
    try:
        with tempfile.TemporaryDirectory(prefix="auth-server-baseline-") as scratch:
            scratch = Path(scratch)
            archive = scratch / "main.tar"
            subprocess.run(["git", "archive", source, "-o", str(archive)], cwd=baseline, check=True)
            tree = scratch / "main"
            tree.mkdir()
            with tarfile.open(archive) as data:
                data.extractall(tree, filter="data")
            tests = tree / "crates/sparq-server/tests"
            (tests / "support").mkdir(exist_ok=True)
            (tests / "fixtures").mkdir(exist_ok=True)
            shutil.copy2(Path(__file__).parent / "support/auth_ledger_equivalence.rs", tests / "support")
            (tests / "fixtures/auth-ledger-main.json").write_text("{}\n")
            module = tree / "crates/sparq-server/src/solid_authz.rs"
            with module.open("a") as file:
                file.write('\n#[cfg(test)]\n#[path = "../tests/support/auth_ledger_equivalence.rs"]\nmod auth_ledger_equivalence;\n')
            env = dict(os.environ, AUTH_SERVER_GOLDEN=str(output))
            if "CARGO_TARGET_DIR" not in env:
                raise RuntimeError("set a shared CARGO_TARGET_DIR")
            subprocess.run([
                "cargo", "test", "-p", "sparq-server", "--features",
                "solid-authz,odrl-authz,solid-authz-trust,backup", "--lib",
                "stateless_auth_ledger_equivalence", "--", "--nocapture",
            ], cwd=tree, env=env, check=True)
    finally:
        # Shared targets can otherwise reuse newer archived-source artifacts for the
        # working tree. Restore freshness for every crate whose public seam changed.
        for crate in ["sparq-core", "sparq-solid", "sparq-policy", "sparq-server"]:
            for path in (ROOT / "crates" / crate / "src").rglob("*.rs"):
                path.touch()
    responses = json.loads(output.read_text())
    output.write_text(json.dumps({"source": source, "responses": responses}, indent=2, sort_keys=True)+"\n")
    print(f"Captured origin/main {source}; review {output}")


if __name__ == "__main__":
    main()
