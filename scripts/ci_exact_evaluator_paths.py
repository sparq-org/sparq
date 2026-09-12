#!/usr/bin/env python3
"""[GPT-6] Fail-closed selection for the mandatory exact-evaluator check."""

import os
import re
import subprocess


def relevant_path(path: str) -> bool:
    return path.startswith((
        "zk/sparql-evaluator/", "crates/sparq-core/", "crates/sparq-engine/",
        "crates/sparq-substrate/", "vendor/spargebra/", ".cargo/", "rust-toolchain",
    )) or path in {
        "Cargo.toml", "Cargo.lock", ".github/workflows/zk-exact-evaluator.yml",
        "scripts/ci_exact_evaluator_paths.py",
        "scripts/tests/test_ci_exact_evaluator_paths.py",
    }


def requires_execution(event: str, base: str, head: str) -> bool:
    """Only a successful, explicit irrelevant diff permits skipping execution."""
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
        return any(relevant_path(os.fsdecode(path)) for path in diff.split(b"\0") if path)
    except (OSError, subprocess.SubprocessError):
        return True


def main() -> None:
    required = requires_execution(
        os.environ.get("GITHUB_EVENT_NAME", ""),
        os.environ.get("EXACT_BASE_SHA", ""),
        os.environ.get("EXACT_HEAD_SHA", ""),
    )
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"required={str(required).lower()}\n")
    print("Exact evaluator execution required." if required else
          "Successful diff classification found no exact-evaluator inputs changed.")


if __name__ == "__main__":
    main()
