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
    "vendor/smallstr-shim/", "vendor/spargebra/", "vendor/spargebra-shim/", "vendor/zk-sdk/", ".cargo/",
    "rust-toolchain",
    ".github/actions/zk-exact-setup/",
)
EXACT_FILES = {
    "Cargo.toml", "Cargo.lock", ".github/workflows/zk-exact-evaluator.yml",
    "scripts/ci_exact_evaluator_paths.py",
    "scripts/ci_exact_evaluator_evidence.py",
    "scripts/tests/test_ci_exact_evaluator_evidence.py",
    "scripts/tests/test_ci_exact_evaluator_paths.py",
    # Imported by test_ci_exact_evaluator_paths.py, which this job runs.
    "scripts/tests/_workflow_trigger_paths.py",
    "crates/sparq-conformance/examples/proof_corpus.rs",
    # Read by the job's bench/zk-bindings regression tests.
    "bench/differential-divergences.json", "crates/sparq-bench/src/fuzz.rs",
    ".github/workflows/zk-toolchain.yml",
}
# Pull requests cover engine, core and substrate as well as the evaluator inputs.
# Root workspace manifests and locks retain their broader push-trigger coverage.
PR_PREFIXES = (
    "zk/sparql-evaluator/", "bench/zk-bindings/", "crates/sparq-canon/",
    "crates/sparq-engine/", "crates/sparq-core/", "crates/sparq-substrate/",
    "crates/sparq-query-protocol/", "vendor/spargebra/", "vendor/spargebra-shim/",
    "vendor/zk-sdk/", ".cargo/", "rust-toolchain", ".github/actions/zk-exact-setup/",
)
PR_FILES = (EXACT_FILES - {"Cargo.toml", "Cargo.lock"}) | {
    "crates/sparq-engine/tests/fixtures/builtin_edges.json",
    "crates/sparq-engine/tests/fixtures/exact_temporal.json",
    "crates/sparq-engine/tests/fixtures/temporal_lexical.json",
}
SCOPES = ("exact-evaluator",)
# On pull requests these dependencies need the executor check alone. Other events
# retain the full evaluator input set for both jobs.
EXECUTOR_ONLY_PR_PREFIXES = (
    "crates/sparq-engine/", "crates/sparq-core/", "crates/sparq-substrate/",
)


def relevant_path(path: str, scope: str = "exact-evaluator") -> bool:
    if scope not in SCOPES:
        raise ValueError(f"unknown selection scope: {scope!r}")
    return path.startswith(EXACT_PREFIXES) or path in EXACT_FILES


def execution_requirements(event: str, base: str, head: str,
                           scope: str = "exact-evaluator") -> tuple[bool, bool]:
    """Return executor/proof requirements, failing closed on uncertain diffs."""
    if scope not in SCOPES:
        raise ValueError(f"unknown selection scope: {scope!r}")
    if event not in {"pull_request", "merge_group", "push"}:
        return True, True
    if not all(re.fullmatch(r"[0-9a-fA-F]{40}", sha) and set(sha) != {"0"}
               for sha in (base, head)):
        return True, True
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
            return True, True
        paths = [os.fsdecode(path) for path in diff.split(b"\0") if path]
        required = any(relevant_path(path, scope) for path in paths)
        proof_required = required
        if event == "pull_request":
            proof_required = any(relevant_path(path, scope) and
                                 not path.startswith(EXECUTOR_ONLY_PR_PREFIXES) for path in paths)
        return required, proof_required
    except (OSError, subprocess.SubprocessError):
        return True, True


def requires_execution(event: str, base: str, head: str, scope: str = "exact-evaluator") -> bool:
    """Whether the diff requires the executor check."""
    return execution_requirements(event, base, head, scope)[0]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scope", choices=SCOPES, default="exact-evaluator",
                        help="job whose inputs are classified (default: exact-evaluator)")
    scope = parser.parse_args().scope
    required, proof_required = execution_requirements(
        os.environ.get("GITHUB_EVENT_NAME", ""),
        os.environ.get("EXACT_BASE_SHA", ""),
        os.environ.get("EXACT_HEAD_SHA", ""),
        scope,
    )
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"required={str(required).lower()}\n")
        output.write(f"proof_required={str(proof_required).lower()}\n")
    print(f"{scope} execution required." if required else
          f"Successful diff classification found no {scope} inputs changed.")


if __name__ == "__main__":
    main()
