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


class QueryProfiles(unittest.TestCase):
    def test_hosted_keeps_agreement_authentication_and_native_verification(self):
        required = {
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
        self.assertEqual(profile.check_noir_listing("\n".join(names)), len(names))
        for missing in (0, len(names) - 1, names.index("sparq_noir_query_core ops_core_reads::reads_check_every_active_input_row")):
            with self.subTest(missing=missing), self.assertRaises(ValueError):
                profile.check_noir_listing("\n".join(names[:missing] + names[missing + 1:] ))
        with self.assertRaises(ValueError):
            profile.check_noir_listing("")

    def test_hosted_noir_filters_reach_every_declared_contract(self):
        names = {f"ops_core_tests::{op}_{case}" for op in profile.OPERATORS
                 for case in ("honest", "missing", "extra", "reordered", "padding", "overflow")}
        names |= {f"ops_core_tests::{name}" for name in profile.ROWS_CONTRACTS}
        for name in names:
            self.assertTrue(any(selected in name for selected in profile.NOIR_FILTERS), name)

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


if __name__ == "__main__":
    unittest.main()
