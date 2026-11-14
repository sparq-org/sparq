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
    "tests/authentication.rs::canonical_envelope_executes_complete_private_evidence_and_policy_controls",
    "tests/authentication.rs::converted_credential_view_cannot_be_witnessed_or_constructed",
)
OPERATORS = ("scan", "join", "filter", "project", "distinct", "slice", "values", "ask", "select")
ROWS_CONTRACTS = (
    "identity_named_view_excludes_default_assertions_and_inactive_rows",
    "rows_plain_string_numeric_class_is_not_a_numeric_value",
    "rows_plain_string_range_class_still_compares_identity",
    "expression_string_ordering_keeps_checked_payloads",
    "rows_plain_string_identity_ignores_unused_key_classes",
    "rows_join_excludes_inactive_source_pairs",
    "rows_join_preserves_bag_multiplicity",
    "rows_join_cannot_reuse_an_output_occurrence",
    "rows_join_cannot_emit_a_pair_twice",
    "rows_join_preserves_duplicate_payload_order",
    "rows_join_refuses_swapped_duplicate_payloads",
    "rows_join_shared_bindings_keep_the_left_payload",
    "rows_join_unbound_compatibility_retains_the_bound_cell",
    "rows_join_skips_an_incompatible_pair_before_a_match",
    "rows_join_incompatible_pairs_cannot_contribute",
    "rows_join_authenticates_the_output_identity",
    "rows_join_authenticates_the_key_class",
    "rows_join_authenticates_the_key",
    "rows_join_authenticates_the_datatype",
    "rows_join_authenticates_the_language",
    "rows_join_covers_a_full_left_input_bucket",
    "rows_permutation_refuses_output_capacity",
    "rows_ordered_projection_refuses_output_capacity",
    "rows_ordered_projection_preserves_duplicate_payload_order",
    "rows_ordered_projection_refuses_swapped_payloads",
    "rows_ordered_projection_refuses_a_changed_identity",
    "rows_ordered_projection_requires_canonical_order",
    "rows_ordered_projection_checks_input_padding",
    "rows_ordered_projection_refuses_missing_rows",
    "rows_ordered_projection_refuses_extra_rows",
    "rows_filter_preserves_selected_duplicate_payloads",
    "rows_filter_refuses_a_different_duplicate_payload",
    "rows_filter_preserves_a_full_input_with_a_smaller_output_bucket",
    "rows_filter_authenticates_the_key_class",
    "rows_filter_authenticates_the_key",
    "rows_filter_authenticates_the_datatype",
    "rows_filter_authenticates_the_language",
    "rows_filter_authenticates_the_identity",
    "rows_filter_authenticates_the_kind",
    "rows_filter_authenticates_the_scope",
    "rows_filter_authenticates_the_bound_flag",
    "rows_filter_checks_input_padding",
    "rows_projection_preserves_stable_duplicate_payloads",
    "rows_projection_refuses_swapped_duplicate_payloads",
    "rows_projection_sorts_the_complete_bucket",
    "rows_permutation_preserves_multiplicity",
    "rows_permutation_refuses_missing_rows",
    "rows_permutation_refuses_extra_rows",
    "rows_permutation_checks_output_order",
    "rows_permutation_checks_input_padding",
    "rows_permutation_checks_output_padding",
    "rows_scan_keeps_the_first_duplicate_payload",
    "rows_scan_authenticates_the_key_class",
    "rows_scan_authenticates_the_key",
    "rows_scan_authenticates_the_datatype",
    "rows_scan_authenticates_the_language",
    "rows_scan_refuses_duplicate_output_bindings",
    "rows_scan_cannot_truncate_a_full_bucket",
    "rows_temporal_scalar_matches_the_existing_byte_extraction",
    "rows_temporal_scalar_refuses_nonzero_tail",
    "rows_temporal_scalar_refuses_a_different_header",
    "rows_temporal_scalar_refuses_an_overflowing_word",
)
POLICY_CONTRACTS = (
    'selected_policy_retains_zero_disabled_words',
    'selected_policy_cannot_drop_requested_holder',
    'selected_policy_cannot_drop_requested_status',
    'selected_policy_cannot_drop_requested_validity',
    'selected_policy_cannot_drop_requested_freshness',
    'selected_policy_cannot_enable_an_unrequested_check',
    'selected_policy_cannot_hide_a_disabled_status_root',
    'selected_policy_cannot_hide_a_disabled_validity_time',
    'selected_policy_cannot_hide_a_disabled_freshness_time',
    'selected_policy_all_consumers_remain_required',
    'disabled_holder_consumer_refuses_signature_evidence',
    'selected_status_cannot_accept_a_false_public_flag',
    'selected_holder_consumer_checks_a_required_signature',
    'selected_validity_cannot_accept_a_false_public_flag',
    'selected_freshness_cannot_accept_a_false_public_flag',
    'status_property_selection_returns_the_authenticated_object',
    'status_property_selection_refuses_another_subject_with_the_same_value',
    'status_property_selection_refuses_another_predicate_with_the_same_value',
    'status_property_selection_refuses_a_named_graph',
    'status_property_selection_refuses_an_unsigned_row',
    'selected_policy_refuses_an_unknown_feature_tag',
    'selected_holder_refuses_an_unknown_feature_tag',
)
NOIR_FILTERS = tuple(f"ops_core_tests::{op}_" for op in OPERATORS) + (
    "ops_core_tests::rows_", "ops_core_reads::", "ops_core_suite::table_identity_", "ops_core_tests::identity_",
    "ops_core_tests::expression_temporal_key_",
    "ops_core_tests::expression_same_type_strings_compare_authenticated_identity",
    "ops_core_tests::expression_string_ordering_keeps_checked_payloads",
    "ops_core_envelope::",
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
    required |= {f"ops_core_tests::{name}" for name in ROWS_CONTRACTS}
    if not required <= names or not any(n.startswith("ops_core_reads::") for n in names):
        raise ValueError("hosted Noir contract inventory is incomplete")
    return len(names)


def check_policy_listing(output: str) -> None:
    names = {line.split()[-1] for line in output.splitlines() if len(line.split()) == 2}
    required = {f"ops_core_envelope::{name}" for name in POLICY_CONTRACTS}
    if not required <= names:
        raise ValueError("hosted policy contract inventory is incomplete")


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
            check_policy_listing(listing.stdout)
        subprocess.run(command, cwd=cwd, env=env, check=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
