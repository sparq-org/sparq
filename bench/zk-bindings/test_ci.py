"""[GPT-6] CI coverage denominators cannot silently shrink or count API refusals."""
import copy
from pathlib import Path
import unittest

from ci import check_inventory


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
