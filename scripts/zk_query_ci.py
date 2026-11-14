#!/usr/bin/env python3
"""Explicit hosted and dispatch profiles for the query toolchain lane."""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import subprocess

QUERY_PREFIX = "crates/sparq-zk-query/"
HOSTED_TESTS = (
    "tests/coverage.rs::compiler_agreement_for_every_passing_plan",
    "tests/proofs.rs::native_proof_reconstructs_inputs_and_consumes_only_after_acceptance",
    "tests/authentication.rs::suite_program_checks_authentication_membership_keys_and_credential_limits",
    "tests/authentication.rs::suite_constant_scan_preserves_literal_identity",
    "tests/authentication.rs::read_site_keys_bind_complete_consumer_inputs_and_lexical_openings",
)
OPERATORS = ("scan", "join", "filter", "project", "distinct", "slice", "values", "ask", "select")
NOIR_FILTERS = tuple(f"ops_core_tests::{op}_" for op in OPERATORS) + (
    "ops_core_reads::", "ops_core_suite::table_identity_", "ops_core_tests::identity_",
    "ops_core_tests::expression_temporal_key_",
    "ops_core_tests::expression_same_type_strings_compare_authenticated_identity",
)


def live_tests(root: Path) -> set[str]:
    result = set()
    for raw in (root / QUERY_PREFIX / "tests/live-tests.txt").read_text().splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        kind, ident = line.split()
        if kind not in {"live", "structural"} or not re.fullmatch(r"(?:tests|src)/[\w/]+\.rs::\w+", ident):
            raise ValueError(f"invalid live inventory row: {line}")
        if kind == "live":
            if ident in result:
                raise ValueError(f"duplicate live test: {ident}")
            result.add(ident)
    if not set(HOSTED_TESTS) <= result:
        raise ValueError("hosted tests must be in the live inventory")
    return result


def steps(root: Path, profile: str) -> list[tuple[list[str], Path]]:
    live = live_tests(root)
    if profile not in {"hosted", "full"}:
        raise ValueError("query execution profile")
    core = root / "zk/noir-query/core"
    smoke = root / "zk/noir-query/fixtures/generated_smoke"
    common = [
        (["node", "zk/noir-query/compiler-agreement/check.mjs", "--self-test"], root),
        (["nargo", "check"], core),
        (["python3", "scripts/check-noir-dependency-aliases.py", "--record-known-failures"], root),
        (["nargo", "check"], smoke),
        (["nargo", "test", "--test-threads", "1"], smoke),
    ]
    if profile == "full":
        return common + [
            (["nargo", "test", "--test-threads", "1"], core),
            (["cargo", "test", "-p", "sparq-zk-query", "--", "--include-ignored", "--test-threads=1"], root),
            (["cargo", "test", "-p", "sparq-zk-query", "--features", "ordered-json", "--", "--test-threads=1"], root),
        ]
    result = common + [
        (["nargo", "test", "--test-threads", "1", *NOIR_FILTERS], core),
        (["cargo", "check", "-p", "sparq-zk-query", "--target", "wasm32-unknown-unknown"], root),
        (["cargo", "check", "-p", "sparq-zk-query", "--target", "wasm32-unknown-unknown", "--features", "ordered-json"], root),
    ]
    skips = [arg for ident in sorted(live) for arg in ("--skip", ident.rsplit("::", 1)[1])]
    for features in ([], ["--features", "ordered-json"]):
        result.append((["cargo", "test", "-p", "sparq-zk-query", *features, "--", "--test-threads=1", *skips], root))
    for ident in HOSTED_TESTS:
        source, test = ident.split("::")
        if not re.fullmatch(r"tests/\w+\.rs", source):
            raise ValueError("hosted integration test target")
        result.append((["cargo", "test", "-p", "sparq-zk-query", "--test", Path(source).stem,
                        test, "--", "--include-ignored", "--exact", "--test-threads=1"], root))
    return result


def check_noir_listing(output: str) -> int:
    names = {line.split()[-1] for line in output.splitlines() if len(line.split()) == 2}
    required = {f"ops_core_tests::{op}_{case}" for op in OPERATORS
                for case in ("honest", "missing", "extra", "reordered", "padding", "overflow")}
    if not required <= names or not any(n.startswith("ops_core_reads::") for n in names):
        raise ValueError("hosted Noir contract inventory is incomplete")
    return len(names)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", choices=("hosted", "full"))
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--list", action="store_true")
    args = parser.parse_args()
    root = args.root.resolve()
    commands = steps(root, args.profile)
    if args.list:
        for command, cwd in commands:
            print(cwd.relative_to(root), *command)
        return 0
    if os.environ.get("SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN") not in (None, "", "0"):
        raise ValueError("query toolchain profile requires live tools")
    # Nested expression tests inherit this CPU affinity. Their default nargo
    # thread count then remains serial without modifying owned fixtures or tools.
    os.sched_setaffinity(0, {min(os.sched_getaffinity(0))})
    env = os.environ.copy()
    env["RAYON_NUM_THREADS"] = "1"
    env["SPARQ_NOIR_QUERY_PROOF_PROFILE"] = "full" if args.profile == "full" else "smoke"
    if args.profile == "full":
        env.pop("SPARQ_NOIR_QUERY_SEED", None)
    else:
        env["SPARQ_NOIR_QUERY_SEED"] = "0"
    for command, cwd in commands:
        print("Running", cwd.relative_to(root), *command, flush=True)
        if command[:2] == ["nargo", "test"] and cwd == root / "zk/noir-query/core" and args.profile == "hosted":
            listing = subprocess.run(["nargo", "test", "--list-tests", *NOIR_FILTERS], cwd=cwd, env=env,
                                     text=True, capture_output=True, check=True)
            check_noir_listing(listing.stdout)
        subprocess.run(command, cwd=cwd, env=env, check=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
