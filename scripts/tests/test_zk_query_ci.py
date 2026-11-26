#!/usr/bin/env python3
"""Behavioral controls for the hosted and dispatch query profiles."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import zk_query_ci as profile

spec = importlib.util.spec_from_file_location("query_gate_checker", ROOT / "scripts/check-zk-toolchain-gate.py")
gate = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = gate
spec.loader.exec_module(gate)

OPENING_AND_EXPRESSION_TESTS = (
    "suite_metadata_tests::digest_default_graph_is_canonical",
    "suite_metadata_tests::digest_opening_authenticates_the_same_signed_tree",
    "suite_metadata_tests::digest_opening_cannot_change_a_term",
    "suite_metadata_tests::digest_opening_cannot_change_a_term_kind",
    "suite_metadata_tests::digest_padding_row_is_canonical",
    "suite_metadata_tests::digest_predicate_must_be_an_iri",
    "suite_metadata_tests::digest_subject_cannot_be_the_default_graph",
    "expr_tests::capacity_and_error_channels_remain_distinct",
    "expr_tests::conditional_and_coalesce",
    "expr_tests::empty_membership_capacity_root_rejected",
    "expr_tests::literal_dispatch_matches_recommendation",
    "expr_binding_tests::bind_error_is_unbound",
    "expr_binding_tests::computed_numeric_canonical_lexical",
)


class QueryProfiles(unittest.TestCase):
    def test_hosted_keeps_agreement_authentication_and_native_verification(self):
        required = {
            "tests/execution.rs::indexed_scan_hints_execute_in_each_witness_layout",
            "tests/coverage.rs::compiler_agreement_for_every_passing_plan",
            "tests/proofs.rs::native_proof_reconstructs_inputs_and_consumes_only_after_acceptance",
            "tests/authentication.rs::suite_program_checks_authentication_membership_keys_and_credential_limits",
            "tests/authentication.rs::suite_constant_scan_preserves_literal_identity",
            "tests/authentication.rs::read_site_keys_bind_complete_consumer_inputs_and_lexical_openings",
            "tests/authentication.rs::canonical_envelope_executes_complete_private_evidence_and_policy_controls",
            "tests/authentication.rs::converted_credential_view_cannot_be_witnessed_or_constructed",
        }
        self.assertEqual(set(profile.HOSTED_TESTS), required)
        hosted = profile.steps(ROOT, "hosted")
        selected = [cmd for cmd, _ in hosted if "--exact" in cmd]
        self.assertEqual({cmd[cmd.index("--test") + 2] for cmd in selected},
                         {ident.rsplit("::", 1)[1] for ident in required})
        for cmd in selected:
            self.assertIn("--include-ignored", cmd)
            self.assertIn("--test-threads=1", cmd)

    def test_dependency_alias_audit_follows_committed_manifest_loading(self):
        for mode in ("hosted", "full"):
            commands = profile.steps(ROOT, mode)
            load = commands.index((["nargo", "check"], ROOT / "zk/noir-query/core"))
            audit = next(i for i, (cmd, _) in enumerate(commands) if "scripts/check-noir-dependency-aliases.py" in cmd)
            self.assertLess(load, audit)

    def test_default_host_tests_defer_every_live_body_explicitly(self):
        live = profile.live_tests(ROOT)
        for cmd, _ in profile.steps(ROOT, "hosted"):
            if cmd[:2] != ["cargo", "test"] or "--exact" in cmd:
                continue
            skipped = {cmd[i + 1] for i, word in enumerate(cmd) if word == "--skip"}
            self.assertEqual(skipped, {ident.rsplit("::", 1)[1] for ident in live})
            self.assertNotIn("--include-ignored", cmd)

    def test_full_sweep_runs_every_live_body_and_complete_core(self):
        full = profile.steps(ROOT, "full")
        self.assertTrue(any(cmd == ["nargo", "test", "--test-threads", "1"]
                            and cwd == ROOT / "zk/noir-query/core" for cmd, cwd in full))
        self.assertTrue(any(cmd[:2] == ["cargo", "test"] and "--include-ignored" in cmd
                            and "--skip" not in cmd and "--exact" not in cmd for cmd, _ in full))
        with self.assertRaises(ValueError):
            profile.steps(ROOT, "unknown")

    def test_hosted_core_keeps_every_operator_contract_case(self):
        names = [f"sparq_noir_query_core ops_core_tests::{op}_{case}" for op in profile.OPERATORS
                 for case in ("honest", "missing", "extra", "reordered", "padding", "overflow")]
        names += ["sparq_noir_query_core ops_core_reads::reads_check_every_active_input_row"]
        names += [f"sparq_noir_query_core ops_core_tests::{name}" for name in profile.ROWS_CONTRACTS]
        names += [f"sparq_noir_query_core ops_core_scan_tests::{name}" for name in profile.INDEXED_SCAN_CONTRACTS]
        names += [f"sparq_noir_query_core {name}" for name in OPENING_AND_EXPRESSION_TESTS]
        self.assertEqual(profile.check_noir_listing("\n".join(names)), len(names))
        for missing in range(len(names)):
            with self.subTest(missing=missing), self.assertRaises(ValueError):
                profile.check_noir_listing("\n".join(names[:missing] + names[missing + 1:]))
        with self.assertRaises(ValueError):
            profile.check_noir_listing("")

    def test_hosted_core_runs_opening_and_expression_controls(self):
        core = ROOT / "zk/noir-query/core"
        command, = (cmd for cmd, cwd in profile.steps(ROOT, "hosted")
                    if cwd == core and cmd[:2] == ["nargo", "test"])
        for name in OPENING_AND_EXPRESSION_TESTS:
            with self.subTest(name=name):
                self.assertTrue(any(selected in name for selected in command[4:]), command)

    def test_hosted_noir_filters_reach_every_declared_contract(self):
        names = {f"ops_core_tests::{op}_{case}" for op in profile.OPERATORS
                 for case in ("honest", "missing", "extra", "reordered", "padding", "overflow")}
        names |= {f"ops_core_tests::{name}" for name in profile.ROWS_CONTRACTS}
        names |= {f"ops_core_scan_tests::{name}" for name in profile.INDEXED_SCAN_CONTRACTS}
        for name in names:
            self.assertTrue(any(selected in name for selected in profile.NOIR_FILTERS), name)

    def test_hosted_indexed_scan_rejections_cannot_disappear(self):
        expected = {
            "indexed_scan_rejects_a_dropped_match",
            "indexed_scan_rejects_a_constant_match_in_output_padding",
            "indexed_scan_rejects_an_extra_row",
            "indexed_scan_rejects_a_wrong_payload_source",
            "indexed_scan_rejects_a_later_source_with_consistent_payload",
            "indexed_scan_rejects_wrong_but_inverse_hints",
            "indexed_scan_accepts_zero_output_capacity_without_matches",
            "indexed_scan_rejects_a_match_with_zero_output_capacity",
            "indexed_scan_rejects_an_extra_row_with_an_empty_dataset",
            "indexed_scan_rejects_a_later_source_after_nonmatching_rows",
            "indexed_scan_rejects_a_repeated_variable_mismatch_with_the_correct_projection",
        }
        self.assertEqual(set(profile.INDEXED_SCAN_CONTRACTS), expected)
        names = {f"ops_core_tests::{op}_{case}" for op in profile.OPERATORS
                 for case in ("honest", "missing", "extra", "reordered", "padding", "overflow")}
        names |= {f"ops_core_tests::{name}" for name in profile.ROWS_CONTRACTS}
        names.add("ops_core_reads::reads_check_every_active_input_row")
        names.update(OPENING_AND_EXPRESSION_TESTS)
        guards = {f"ops_core_scan_tests::{name}" for name in expected}
        names |= guards
        self.assertEqual(profile.check_noir_listing("\n".join(f"sparq_noir_query_core {n}" for n in names)), len(names))
        for guard in guards:
            with self.subTest(guard=guard), self.assertRaises(ValueError):
                profile.check_noir_listing("\n".join(f"sparq_noir_query_core {n}" for n in names - {guard}))

    def test_ledger_profiles_partition_the_real_inventory(self):
        trees = gate.collect(ROOT)
        sites, failures = gate.inventory(trees)
        self.assertFalse(failures)
        all_keys = gate.profile_keys(ROOT, sites, "all")
        estate = gate.profile_keys(ROOT, sites, "estate")
        full = gate.profile_keys(ROOT, sites, "query-full")
        hosted = gate.profile_keys(ROOT, sites, "query-hosted")
        package = gate.profile_keys(ROOT, sites, "query-package")
        self.assertTrue(full and hosted)
        self.assertFalse(estate & full)
        self.assertFalse(package & (estate | full))
        self.assertEqual(estate | full | package, all_keys)
        self.assertLessEqual(hosted, full)
        self.assertEqual(len(hosted), len(profile.HOSTED_TESTS))
        with self.assertRaises(ValueError):
            gate.profile_keys(ROOT, sites, "unknown")

    def test_dispatch_condition_and_ledger_steps_cannot_be_weakened(self):
        import copy
        import yaml
        document = yaml.safe_load((ROOT / ".github/workflows/zk-toolchain.yml").read_text())
        self.assertFalse(gate.check_query_workflow(ROOT, document))
        for name in ("query-hosted", "query-full"):
            for kind in ("DEV", "TEST"):
                self.assertEqual(document["jobs"][name]["env"][f"CARGO_PROFILE_{kind}_DEBUG"], "0")
        mutants = []
        for name in ("query-hosted", "query-full"):
            changed = copy.deepcopy(document); del changed["jobs"][name]; mutants.append(changed)
            changed = copy.deepcopy(document); changed["jobs"][name]["continue-on-error"] = True; mutants.append(changed)
            changed = copy.deepcopy(document); changed["jobs"][name]["steps"][-1]["if"] = "always()"; mutants.append(changed)
            changed = copy.deepcopy(document); changed["jobs"][name]["steps"][-1]["run"] += " || true"; mutants.append(changed)
            changed = copy.deepcopy(document); changed["jobs"][name]["env"].clear(); mutants.append(changed)
        changed = copy.deepcopy(document); changed["jobs"]["query-full"]["if"] = "always()"; mutants.append(changed)
        changed = copy.deepcopy(document)
        step = next(s for s in changed["jobs"]["query-hosted"]["steps"] if s.get("name") == "Run hosted query profile")
        step["run"] = "python3 scripts/zk_query_ci.py full"; mutants.append(changed)
        for changed in mutants:
            self.assertTrue(gate.check_query_workflow(ROOT, changed))

    def test_hosted_ledger_requires_every_entry_and_completion(self):
        import contextlib
        import io
        sites, failures = gate.inventory(gate.collect(ROOT))
        self.assertFalse(failures)
        keys = sorted(gate.profile_keys(ROOT, sites, "query-hosted"))
        complete = gate._ledger_lines(keys)
        with tempfile.TemporaryDirectory() as tmp:
            ledger = Path(tmp) / "ledger.txt"
            for lines, selected, expected in (
                (complete, "query-hosted", 0),
                (complete, "query-full", 1),
                (complete, "estate", 1),
                (complete[:-1], "query-hosted", 1),
                (complete[1:], "query-hosted", 1),
                (complete + ["enter crates/sparq-zk-query/tests/absent.rs:1"], "query-hosted", 1),
            ):
                ledger.write_text("\n".join(lines) + "\n")
                with self.subTest(selected=selected, lines=lines), contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    self.assertEqual(gate.verify_ledger(ROOT, ledger, selected), expected)


class QueryPolicyFeatures(unittest.TestCase):
    def policy_listing(self):
        return [f"sparq_noir_query_core ops_core_envelope::{name}" for name in profile.POLICY_CONTRACTS]

    def test_policy_inventory_keeps_the_consumer_guards(self):
        required = {
            "selected_policy_cannot_drop_requested_holder",
            "selected_policy_refuses_an_unknown_feature_tag",
            "selected_holder_refuses_an_unknown_feature_tag",
            "selected_status_cannot_accept_a_false_public_flag",
            "selected_validity_cannot_accept_a_false_public_flag",
            "selected_freshness_cannot_accept_a_false_public_flag",
            "selected_policy_cannot_hide_a_disabled_status_root",
            "selected_policy_cannot_hide_a_disabled_validity_time",
            "selected_policy_cannot_hide_a_disabled_freshness_time",
            "selected_holder_consumer_checks_a_required_signature",
            "disabled_holder_consumer_refuses_signature_evidence",
            "status_property_selection_refuses_an_unsigned_row",
            "status_property_selection_refuses_another_subject_with_the_same_value",
            "status_property_selection_refuses_another_predicate_with_the_same_value",
            "status_property_selection_refuses_a_named_graph",
        }
        self.assertLessEqual(required, set(profile.POLICY_CONTRACTS))

    def test_policy_listing_requires_every_flag_and_source_control(self):
        names = self.policy_listing()
        profile.check_policy_listing("\n".join(names))
        for missing in range(len(names)):
            with self.subTest(missing=missing), self.assertRaises(ValueError):
                profile.check_policy_listing("\n".join(names[:missing] + names[missing + 1:]))

    def test_hosted_filters_reach_the_policy_controls(self):
        for name in profile.POLICY_CONTRACTS:
            qualified = f"ops_core_envelope::{name}"
            self.assertTrue(any(selected in qualified for selected in profile.NOIR_FILTERS), qualified)

    def test_hosted_execution_checks_its_core_listing_before_running(self):
        from contextlib import redirect_stdout
        import io
        import os
        import subprocess
        from unittest import mock
        names = [f"sparq_noir_query_core ops_core_tests::{op}_{case}" for op in profile.OPERATORS
                 for case in ("honest", "missing", "extra", "reordered", "padding", "overflow")]
        names += ["sparq_noir_query_core ops_core_reads::reads_check_every_active_input_row"]
        names += [f"sparq_noir_query_core ops_core_tests::{name}" for name in profile.ROWS_CONTRACTS]
        names += [f"sparq_noir_query_core ops_core_scan_tests::{name}" for name in profile.INDEXED_SCAN_CONTRACTS]
        names += [f"sparq_noir_query_core {name}" for name in OPENING_AND_EXPRESSION_TESTS]
        names += self.policy_listing()
        core = ROOT / "zk/noir-query/core"
        for missing, other_package in (
            (None, False),
            ("ops_core_scan_tests::indexed_scan_rejects_a_constant_match_in_output_padding", True),
            ("ops_core_scan_tests::indexed_scan_rejects_a_wrong_payload_source", False),
            ("suite_metadata_tests::digest_opening_cannot_change_a_term_kind", False),
            ("expr_binding_tests::bind_error_is_unbound", False),
            ("expr_binding_tests::bind_error_is_unbound", True),
            ("ops_core_envelope::selected_holder_refuses_an_unknown_feature_tag", False),
            ("ops_core_envelope::selected_holder_refuses_an_unknown_feature_tag", True),
        ):
            missing_row = f"sparq_noir_query_core {missing}"
            listing = [name for name in names if name != missing_row]
            if other_package:
                listing.append(f"other_package {missing}")
            with self.subTest(missing=missing, other_package=other_package), \
                 redirect_stdout(io.StringIO()), \
                 mock.patch.object(sys, "argv", ["zk_query_ci.py", "hosted", "--root", str(ROOT)]), \
                 mock.patch.dict(os.environ, {"SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN": "0"}), \
                 mock.patch.object(profile.os, "sched_setaffinity"), \
                 mock.patch.object(profile.subprocess, "run", return_value=subprocess.CompletedProcess(
                     [], 0, stdout="\n".join(listing))) as run:
                if missing is None:
                    self.assertEqual(profile.main(), 0)
                else:
                    with self.assertRaisesRegex(ValueError, "contract inventory is incomplete"):
                        profile.main()
                core_calls = [call for call in run.call_args_list
                              if call.kwargs["cwd"] == core and call.args[0][:2] == ["nargo", "test"]]
                self.assertEqual(len(core_calls), 2 if missing is None else 1)
                listing_call = core_calls[0]
                self.assertIn("--list-tests", listing_call.args[0])
                self.assertTrue(listing_call.kwargs["check"])
                if missing is None:
                    execution_call = core_calls[1]
                    self.assertEqual(listing_call.args[0], [*execution_call.args[0], "--list-tests"])
                    self.assertNotIn("--no-run", execution_call.args[0])
                    self.assertTrue(execution_call.kwargs["check"])
                else:
                    self.assertEqual(run.call_args, listing_call)


if __name__ == "__main__":
    unittest.main()
