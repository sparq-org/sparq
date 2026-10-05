#!/usr/bin/env python3
"""[GPT-6] Fail-closed selection for the mandatory native-composition check."""

import os
import re
import subprocess


def relevant_path(path: str) -> bool:
    return path.startswith((
        "zk/native-composition/", "crates/sparq-canon/", "crates/sparq-core/",
        "vendor/spargebra/", ".cargo/", "rust-toolchain",
    )) or path in {
        "Cargo.toml", ".github/workflows/zk-native-composition.yml",
        "scripts/ci_native_composition_paths.py",
        "scripts/tests/test_ci_native_composition_paths.py",
        "scripts/ci_summary_gate.py", "scripts/tests/test_ci_summary_gate.py",
        ".github/workflows/ci-summary.yml",
        "bench/zk-bindings/corpus.py", "bench/zk-bindings/run.py",
        "bench/zk-bindings/protocol.json", "bench/zk-bindings/inventory.json",
        "bench/zk-bindings/native_ci.py", "bench/zk-bindings/test_native_ci.py",
        "bench/zk-bindings/test_harness.py",
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
        os.environ.get("NATIVE_BASE_SHA", ""),
        os.environ.get("NATIVE_HEAD_SHA", ""),
    )
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"required={str(required).lower()}\n")
    print("Native composition execution required." if required else
          "Successful diff classification found no native-composition inputs changed.")


if __name__ == "__main__":
    main()
