#!/usr/bin/env python3
"""Capture authorization goldens from the pinned main source, never the PR code.

Usage: python3 crates/sparq-solid/tests/regenerate_auth_ledger.py OUTPUT_DIR
Uses a disposable build directory below the caller's CARGO_TARGET_DIR and limits
compilation to two jobs. The archive, build output and observer edits are removed on
exit; expected fixtures are not replaced.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile

SOURCE = "81df9457597ef8b81c384360349c94ef956b5220"
ROOT = Path(__file__).resolve().parents[3]


def main():
    output = Path(sys.argv[1]).resolve()
    output.mkdir(parents=True, exist_ok=True)
    build_parent = Path(os.environ.get("CARGO_TARGET_DIR", tempfile.gettempdir()))
    build_parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="auth-ledger-baseline-") as scratch, \
            tempfile.TemporaryDirectory(prefix="baseline-goldens-", dir=build_parent) as build:
        scratch = Path(scratch)
        archive = scratch / "source.tar"
        subprocess.run(["git", "archive", SOURCE, "-o", str(archive)], cwd=ROOT, check=True)
        source = scratch / "source"
        source.mkdir()
        with tarfile.open(archive) as tree:
            tree.extractall(source, filter="data")
        tests = source / "crates/sparq-solid/tests"
        shutil.copy2(Path(__file__).with_name("auth_ledger_equivalence.rs"), tests)
        for fixture in Path(__file__).with_name("fixtures").glob("auth-ledger-*.json"):
            shutil.copy2(fixture, tests / "fixtures" / fixture.name)
        materialize = ROOT / "crates/sparq-solid/src/materialize.rs"
        observer = materialize.read_text().split(
            "#[cfg(test)]\nstd::thread_local! {\n    static OBSERVED_CHARGES:", 1
        )[1]
        observer = "#[cfg(test)]\nstd::thread_local! {\n    static OBSERVED_CHARGES:" + observer
        target = source / "crates/sparq-solid/src/materialize.rs"
        code = target.read_text()
        seam = "    work.charge(charge)?;"
        assert code.count(seam) == 1, "review the baseline charge-observer seam"
        code = code.replace(seam, "    #[cfg(test)]\n    record_charge(charge);\n" + seam)
        target.write_text(code + "\n" + observer)
        lib = source / "crates/sparq-solid/src/lib.rs"
        with lib.open("a") as file:
            file.write('\n#[cfg(test)]\nextern crate self as sparq_solid;\n#[cfg(test)]\n#[path = "../tests/auth_ledger_equivalence.rs"]\nmod ledger_equivalence;\n')
        env = dict(os.environ, AUTH_LEDGER_GOLDEN_DIR=str(output),
                   RUST_TEST_THREADS="2", CARGO_TARGET_DIR=build)
        subprocess.run([
            "cargo", "test", "-p", "sparq-solid", "--all-features", "--lib", "baseline", "-j", "2"
        ], cwd=source, env=env, check=True)
        (output / "source-revision.txt").write_text(SOURCE + "\n")


if __name__ == "__main__":
    main()
