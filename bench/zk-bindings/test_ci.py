"""[GPT-6] CI coverage denominators cannot silently shrink or count API refusals."""
import copy
from pathlib import Path
import unittest

from ci import PUBLIC_PATTERN_NATIVE, check_inventory, check_public_pattern_scopes


class CiTests(unittest.TestCase):
    def setUp(self):
        self.expected = {"noir_unsigned": (46, 4, 42)}
        self.report = {"passed":True, "complete_declared_domain":True, "totals":{
            "noir_unsigned":{"configured_jobs":46,"shard_jobs":46,"executed_jobs":46,"passed_jobs":46,
                             "genuine_proofs":4,"verified_proofs":4,"negative_stages":{"constraint":42}}}}

    def test_exact_executed_counts_and_constraint_tier(self):
        check_inventory(self.report, self.expected)
        for field, value in [("configured_jobs",45),("executed_jobs",45),("passed_jobs",45),
                             ("verified_proofs",0),("genuine_proofs",0),
                             ("negative_stages",{"support":42})]:
            altered = copy.deepcopy(self.report)
            altered["totals"]["noir_unsigned"][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                check_inventory(altered, self.expected)
        self.report["totals"]["unexpected"] = self.report["totals"]["noir_unsigned"]
        with self.assertRaisesRegex(ValueError, "denominator"):
            check_inventory(self.report, self.expected)

    def test_public_pattern_native_scopes_are_exact(self):
        # [OPUS-5.5] beadzkp-15.1.1: counts are re-derived, not copied from a report.
        from itertools import product
        from corpus import exhaustive
        edges = [(s, o) for s in ("a", "b") for o in ("a", "b")]
        derived = {"accepted":0, "empty_graph":0, "native_support":0}
        for mask in range(16):
            present = {e for i, e in enumerate(edges) if mask & (1 << i)}
            for width in (2, 3):
                for row in product("abm", repeat=width):
                    valid = all(pair in present for pair in zip(row, row[1:]))
                    derived["accepted" if valid else "native_support" if present else "empty_graph"] += 1
        self.assertEqual(derived, PUBLIC_PATTERN_NATIVE)
        self.assertEqual(sum(len(c["expected"]["Select"]["rows"]) for c in exhaustive()
                             if c["template"] in ("scan", "join")), 72)
        records = [{"backend":"noir_public_pattern", "outcome":{"observed":"accepted"}}] * 72
        records += [{"backend":"noir_public_pattern", "outcome":{"observed":"rejected", "rejection_scope":"empty_graph"}}] * 36
        records += [{"backend":"noir_public_pattern", "outcome":{"observed":"rejected", "rejection_scope":"native_support"}}] * 468
        records += [{"backend":"noir_unsigned", "outcome":{"observed":"rejected"}}]
        check_public_pattern_scopes({"records":records}, PUBLIC_PATTERN_NATIVE)
        for altered in (records[1:], records + records[:1],
                        records[:72] + [{"backend":"noir_public_pattern", "outcome":{"observed":"rejected"}}] + records[73:]):
            with self.assertRaises(ValueError):
                check_public_pattern_scopes({"records":altered}, PUBLIC_PATTERN_NATIVE)

    def test_generic_sweep_keeps_all_prior_tests_and_required_invocation(self):
        workflow = (Path(__file__).resolve().parents[2] / '.github/workflows/zk-toolchain.yml').read_text()
        command = 'cargo test -p sparq-zk-compose --features successful-results --lib result:: -- --include-ignored --test-threads=1'
        matches = [line.strip() for line in workflow.splitlines() if command in line]
        self.assertEqual(matches, [command + ' --skip result::proof_bindings::run_job'])
        self.assertIn('python3 bench/zk-bindings/ci.py --output', workflow)
        self.assertIn('cargo check --locked -p sparq-conformance --example proof_corpus', workflow)
        self.assertIn('cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator-model --features evaluate --example proof_bindings', workflow)
        self.assertIn('timeout-minutes: 20', workflow)
        self.assertEqual(workflow.count('- "bench/zk-bindings/**"'), 2)
        self.assertEqual(workflow.count('- "zk/sparql-evaluator/model/**"'), 2)
        self.assertEqual(workflow.count('- "crates/sparq-conformance/examples/proof_corpus.rs"'), 2)
        self.assertIn('if-no-files-found: error', workflow)
        self.assertIn('actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a', workflow)


if __name__ == '__main__':
    unittest.main()
