import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import guest_determinism as gate


def record():
    execution = {"user_cycles": 11, "padded_cycles": 16, "segments": 1,
                 "journal_bytes": 4, "witness_bytes": 8, "witness_sha256": "w",
                 "artifact_sha256": "a", "journal_sha256": "j", "execute_ns": 99}
    return {"set": "main", "id": "test", "suite": "eddsa-rdfc-2022", "mode": "revealed",
            "n": 1, "statements_per_credential": 17, "admitted": True,
            "query_profile": "accepted", "execution": execution,
            "phase_execution": copy.deepcopy(execution), "phase_cycles": {"Query": 7},
            "native_journal_sha256": "j"}


class GuestDeterminismTests(unittest.TestCase):
    def test_evidence_workflow_requires_the_executor_gate(self):
        workflow = (gate.ROOT / ".github/workflows/zk-exact-evaluator.yml").read_text()
        job = workflow.split("  exact-evaluator:\n", 1)[1]
        self.assertNotIn("continue-on-error:", job)
        steps = job.split("\n      - ")
        matches = [step for step in steps if step.startswith(
            "name: Guest execution reproducibility and randomness boundary\n")]
        self.assertEqual(len(matches), 1)
        lines = [line.strip() for line in matches[0].splitlines()
                 if line.strip() and not line.strip().startswith("#")]
        self.assertEqual(lines[1], "if: steps.changes.outputs.required == 'true'")
        self.assertEqual(lines[2:], [
            "run: |", "set -euo pipefail", "sudo apt-get update",
            "sudo apt-get install --yes gdb binutils",
            "python3 -m unittest discover -s zk/sparql-evaluator/scripts -p 'test_guest_determinism.py'",
            'python3 zk/sparql-evaluator/scripts/guest_determinism.py --output "$RUNNER_TEMP/sparq-guest-determinism"',
        ])

    def test_general_operator_paths_cannot_be_replaced_by_index_shortcuts(self):
        path = gate.ROOT / "zk/sparql-evaluator/fixtures/paper/determinism.json"
        cases = json.loads(path.read_text())["cases"]
        self.assertEqual({case["id"] for case in cases}, {
            "distinct-values", "count-distinct-values", "concat-distinct-values",
            "cyclic-variable-bgp", "canonical-blank-table", "regex-replace-captures",
        })
        self.assertIn("VALUES", next(c["query"] for c in cases if c["id"] == "distinct-values"))

    def test_startup_entropy_samples_are_distinct_and_complete(self):
        self.assertEqual(len(gate.SAMPLES), 5)
        self.assertEqual(len(set(gate.SAMPLES)), 5)
        rows = [{"startup_pattern": seed, "startup_entropy_sha256": str(seed),
                 "randomness_calls": 1, "error": None, "exit_code": 0} for seed in gate.SAMPLES]
        gate.validate_observations(rows, 1, gate.SAMPLES)
        for field, value in (("startup_entropy_sha256", "constant"), ("randomness_calls", 2),
                             ("exit_code", None), ("error", "failed")):
            changed = copy.deepcopy(rows)
            for row in changed:
                row[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                gate.validate_observations(changed, 1, gate.SAMPLES)
        with self.assertRaises(ValueError):
            gate.validate_observations(rows[:-1], 1, gate.SAMPLES)
        repeated = [copy.deepcopy(row) for row in rows for _ in range(2)]
        repeated[1]["startup_entropy_sha256"] = "changed-within-sample"
        with self.assertRaisesRegex(ValueError, "within a sample"):
            gate.validate_observations(repeated, 2, gate.SAMPLES)

    def test_changed_corpus_membership_requires_review(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fixtures = root / "zk/sparql-evaluator/fixtures/paper"
            fixtures.mkdir(parents=True)
            (fixtures / "only.rq").write_text("ASK {}")
            for name in ("sweep", "determinism"):
                (fixtures / f"{name}.json").write_text(json.dumps({"cases": [{"id": "only"}]}))
            canon = root / "crates/sparq-canon/tests/rdf-canon-testdata"
            canon.mkdir(parents=True)
            (canon / "manifest.ttl").write_text("")
            with patch.object(gate, "ROOT", root):
                for campaign in ("main", "sweep", "determinism"):
                    with self.subTest(campaign=campaign), self.assertRaisesRegex(ValueError, "membership changed"):
                        gate.census([], campaign)
                with self.assertRaisesRegex(ValueError, "membership changed"):
                    gate.canonical_inputs()

    def test_unsupported_runtime_fails_before_building_or_executing(self):
        with patch("sys.argv", ["guest_determinism.py", "--output", "/tmp/unused-guest-check"]), \
                patch.dict(gate.os.environ, RISC0_SERVER_PATH="/tmp/unsupported-runtime"), \
                patch.object(gate.subprocess, "check_output", return_value="unsupported runtime"), \
                patch.object(gate.subprocess, "run", side_effect=AssertionError("unexpected execution")):
            with self.assertRaisesRegex(ValueError, "pinned runtime"):
                gate.main()

    def test_canonical_corpus_includes_both_profiles_and_expected_outputs(self):
        inputs = gate.canonical_inputs()
        self.assertEqual(len(inputs), 88)
        self.assertEqual(sum("expected" in item for item in inputs), 64)
        self.assertEqual(sum(item.get("negative", False) for item in inputs), 1)
        self.assertEqual({item["id"] for item in inputs if item["sha384"]}, {"test075c", "test075m"})

    def test_every_count_and_identity_participates_in_comparison(self):
        original = record()
        for section in ("execution", "phase_execution"):
            for field in ("user_cycles", "padded_cycles", "segments", "journal_bytes", "witness_bytes",
                          "witness_sha256", "artifact_sha256", "journal_sha256"):
                changed = copy.deepcopy(original)
                value = changed[section][field]
                changed[section][field] = value + 1 if isinstance(value, int) else value + "changed"
                with self.subTest(section=section, field=field), self.assertRaises(ValueError):
                    gate.compare(gate.snapshot(original), gate.snapshot(changed))
        changed = copy.deepcopy(original)
        changed["phase_cycles"]["Query"] += 1
        with self.assertRaises(ValueError):
            gate.compare(gate.snapshot(original), gate.snapshot(changed))

    def test_timers_do_not_enter_the_count_invariant(self):
        original = record()
        changed = copy.deepcopy(original)
        changed["execution"]["execute_ns"] += 1
        changed["phase_execution"]["execute_ns"] += 1
        changed["native_evaluate_ns"] = 100
        gate.compare(gate.snapshot(original), gate.snapshot(changed))

    def test_native_journal_is_an_independent_semantic_oracle(self):
        changed = record()
        changed["native_journal_sha256"] = "different"
        with self.assertRaises(ValueError):
            gate.snapshot(changed)

    def test_executor_errors_and_incomplete_phase_results_fail(self):
        for section in ("execution", "phase_execution", "phase_cycles"):
            changed = record()
            changed[section] = {"error": "execution failed", "execute_ns": 0}
            with self.subTest(section=section), self.assertRaises(ValueError):
                gate.snapshot(changed)
        changed = record()
        del changed["execution"]["artifact_sha256"]
        with self.assertRaises(ValueError):
            gate.snapshot(changed)

    def test_rejections_require_a_guest_abort_and_no_other_executor_error(self):
        rejected = record()
        rejected.update(admitted=False, guest_aborted=True, rejection="not admitted")
        gate.snapshot(rejected)
        del rejected["guest_aborted"]
        with self.assertRaises(ValueError):
            gate.snapshot(rejected)
        rejected.update(guest_aborted=True, guest_error="session limit")
        with self.assertRaises(ValueError):
            gate.snapshot(rejected)

    def test_full_census_cannot_pass_with_an_omitted_or_duplicate_record(self):
        fixtures = gate.ROOT / "zk/sparql-evaluator/fixtures/paper"
        records = []
        for path in fixtures.glob("*.rq"):
            for suite in ("eddsa-rdfc-2022", "eddsa-sha256-merkle-2026"):
                for mode in ("hidden", "revealed"):
                    for n in (1, 4):
                        item = record()
                        item.update(id=path.stem, suite=suite, mode=mode, n=n)
                        records.append(item)
        gate.census(records, "main")
        with self.assertRaises(ValueError):
            gate.census(records[1:], "main")
        with self.assertRaises(ValueError):
            gate.census(records + records[:1], "main")


if __name__ == "__main__":
    unittest.main()
