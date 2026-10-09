#!/usr/bin/env python3
"""Fail-closed selection for the mandatory exact-evaluator check."""

import argparse
import os
import re
import subprocess

# Local crates must cover evaluator_path_closure() in
# scripts/tests/test_ci_exact_evaluator_paths.py, which derives them from the manifests.
EXACT_PREFIXES = (
    "zk/sparql-evaluator/", "bench/zk-bindings/", "crates/sparq-core/", "crates/sparq-engine/",
    "crates/sparq-substrate/", "crates/sparq-canon/", "crates/sparq-query-protocol/",
    "crates/sparq-engine-serialize/", "crates/sparq-engine-service/", "crates/sparq-introspect/",
    "crates/sparq-jsonld/",
    "vendor/spargebra/", "vendor/spargebra-shim/", "vendor/zk-sdk/", ".cargo/", "rust-toolchain",
)
EXACT_FILES = {
    "Cargo.toml", "Cargo.lock", ".github/workflows/zk-exact-evaluator.yml",
    "scripts/ci_exact_evaluator_paths.py",
    "scripts/ci_exact_evaluator_evidence.py",
    "scripts/tests/test_ci_exact_evaluator_evidence.py",
    "scripts/tests/test_ci_exact_evaluator_paths.py",
    "crates/sparq-conformance/examples/proof_corpus.rs",
    # Read by the job's bench/zk-bindings regression tests.
    "bench/differential-divergences.json", "crates/sparq-bench/src/fuzz.rs",
    ".github/workflows/zk-toolchain.yml",
}
SCOPES = ("exact-evaluator",)


def relevant_path(path: str, scope: str = "exact-evaluator") -> bool:
    if scope not in SCOPES:
        raise ValueError(f"unknown selection scope: {scope!r}")
    return path.startswith(EXACT_PREFIXES) or path in EXACT_FILES


def requires_execution(event: str, base: str, head: str, scope: str = "exact-evaluator") -> bool:
    """Only a successful, explicit irrelevant diff permits skipping execution."""
    if scope not in SCOPES:
        raise ValueError(f"unknown selection scope: {scope!r}")
    if event not in {"pull_request", "merge_group", "push"}:
        return True
    if not all(re.fullmatch(r"[0-9a-fA-F]{40}", sha) and set(sha) != {"0"}
               for sha in (base, head)):
        return True
    try:
        subprocess.run(
            ["git", "fetch", "--no-tags", "--depth=1", "origin", base, head],
            check=True, capture_output=True, timeout=60,
        )
        diff = subprocess.run(
            ["git", "diff", "--name-only", "--no-renames", "-z", base, head, "--"],
            check=True, capture_output=True, timeout=60,
        ).stdout
        # Git's -z output must be complete; ambiguous/truncated output runs tests.
        if diff and not diff.endswith(b"\0"):
            return True
        return any(relevant_path(os.fsdecode(path), scope) for path in diff.split(b"\0") if path)
    except (OSError, subprocess.SubprocessError):
        return True


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scope", choices=SCOPES, default="exact-evaluator",
                        help="job whose inputs are classified (default: exact-evaluator)")
    scope = parser.parse_args().scope
    required = requires_execution(
        os.environ.get("GITHUB_EVENT_NAME", ""),
        os.environ.get("EXACT_BASE_SHA", ""),
        os.environ.get("EXACT_HEAD_SHA", ""),
        scope,
    )
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"required={str(required).lower()}\n")
    print(f"{scope} execution required." if required else
          f"Successful diff classification found no {scope} inputs changed.")


if __name__ == "__main__":
    main()
