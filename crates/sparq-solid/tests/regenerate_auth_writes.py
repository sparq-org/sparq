#!/usr/bin/env python3
"""Capture write-equivalence fixtures from the pinned source, never the checked code.

Usage: python3 crates/sparq-solid/tests/regenerate_auth_writes.py OUTPUT_DIR
Review OUTPUT_DIR/auth-write-ledger.json before replacing the committed fixture.
The archive is temporary; builds reuse the caller's sole CARGO_TARGET_DIR.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile

SOURCE = "c5d9b0387ced6f81e471463e18c08f8bf2f9ba5c"
ROOT = Path(__file__).resolve().parents[3]


def main():
    merge_base = subprocess.check_output(
        ["git", "merge-base", "HEAD", "origin/main"], cwd=ROOT, text=True
    ).strip()
    if merge_base != SOURCE:
        raise RuntimeError("update SOURCE to the merge base before capturing fixtures")
    output = Path(sys.argv[1]).resolve()
    output.mkdir(parents=True, exist_ok=True)
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()
    try:
        with tempfile.TemporaryDirectory(prefix="auth-write-baseline-") as scratch:
            scratch = Path(scratch)
            archive = scratch / "source.tar"
            subprocess.run(["git", "archive", SOURCE, "-o", str(archive)], cwd=ROOT, check=True)
            source = scratch / "source"
            source.mkdir()
            with tarfile.open(archive) as tree:
                tree.extractall(source, filter="data")
            tests = source / "crates/sparq-solid/tests"
            for name in [
                "auth_write_equivalence.rs",
                "write_admission_parity.rs",
                "fixtures/auth-write-ledger.json",
            ]:
                shutil.copy2(Path(__file__).parent / name, tests / name)
            # A shared target directory can otherwise consider the archived source older
            # than artifacts built from the working tree. Force the changed crate fresh.
            for path in (source / "crates/sparq-solid/src").rglob("*.rs"):
                path.touch()
            captures = scratch / "captures"
            env = dict(os.environ, CARGO_TARGET_DIR=str(target),
                       AUTH_WRITE_GOLDEN_DIR=str(captures), RUST_TEST_THREADS="2")
            subprocess.run([
                "cargo", "test", "-p", "sparq-solid", "--all-features", "-j", "2",
                "--test", "auth_write_equivalence", "--", "--include-ignored",
            ], cwd=source, env=env, check=True)
            snapshots = {}
            for path in captures.glob("*.json"):
                capture = json.loads(path.read_text())
                key = capture["key"]
                if key in snapshots:
                    raise RuntimeError(f"duplicate capture: {key}")
                snapshots[key] = capture["snapshot"]
            if not snapshots:
                raise RuntimeError("no writes were captured")
            (output / "auth-write-ledger.json").write_text(json.dumps(
                {"source": SOURCE, "writes": snapshots}, indent=2, sort_keys=True
            ) + "\n")
    finally:
        # Restore freshness for the caller's next build without changing source content.
        for path in (ROOT / "crates/sparq-solid/src").rglob("*.rs"):
            path.touch()


if __name__ == "__main__":
    main()
