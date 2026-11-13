"""Synthetic schema fixtures; no measured results are retained here."""
import copy
import gzip
import json
from pathlib import Path
import tempfile
import unittest

from paper_executor_audit import load, validate_record


def record(merkle=False, hidden=True, proof=False):
    value = dict(
        schema="sparq.zk-paper-sweep.v2", n=1,
        suite="eddsa-sha256-merkle-2026" if merkle else "eddsa-rdfc-2022",
        mode="hidden" if hidden else "revealed",
        issuer_sign_ns=[100], issuer_signature_primitive_ns=[20],
        issuer_merkle_tree_ns=[30 if merkle else None],
        credential_bytes=[1000], credential_encoded_bytes=[2000],
        credential_encoding="serde-json-signed-credential-v1",
        presentation_encoding="serde-json-answer-presentation-v1",
        verifier_split_status="measured" if proof else "not_measured_no_receipt",
    )
    if proof:
        value["proofs"] = [dict(
            verify_ns=100, verifier_receipt_ns=40, verifier_request_ns=10,
            verifier_signature_ns=None if hidden else 20,
            verifier_stages_ns=50 if hidden else 70,
            presentation_bytes=1000, presentation_encoded_bytes=2000,
        )]
    return value


class PaperSchemaTests(unittest.TestCase):
    def test_old_and_new_records_remain_readable(self):
        records = [{"id": "legacy"}, {"schema": "sparq.zk-paper-sweep.v1"}]
        records += [record(m, h, p) for m in (False, True) for h in (False, True) for p in (False, True)]
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "records.jsonl.gz"
            with gzip.open(path, "wt") as out:
                out.write("\n".join(map(json.dumps, records)))
            self.assertEqual(load(path)[1], records)

    def test_incomplete_or_overlapping_measurements_are_rejected(self):
        original = record(merkle=True, hidden=False, proof=True)
        changes = [
            ("schema", "future"), ("issuer_signature_primitive_ns", []),
            ("issuer_merkle_tree_ns", [None]), ("issuer_sign_ns", [0]),
            ("credential_encoded_bytes", [1000]), ("credential_encoding", "components"),
            ("presentation_encoding", "components"), ("verifier_split_status", "not_measured_no_receipt"),
        ]
        for field, bad in changes:
            with self.subTest(field=field), self.assertRaises(ValueError):
                changed = copy.deepcopy(original)
                changed[field] = bad
                validate_record(changed)
        for field, bad in [("verifier_signature_ns", None), ("verifier_receipt_ns", 100),
                           ("verifier_request_ns", -1), ("verifier_stages_ns", 0),
                           ("verify_ns", 0), ("presentation_encoded_bytes", 1000)]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                changed = copy.deepcopy(original)
                changed["proofs"][0][field] = bad
                validate_record(changed)
        changed = record(hidden=True, proof=True)
        changed["proofs"][0]["verifier_signature_ns"] = 0
        with self.assertRaises(ValueError):
            validate_record(changed)
        changed = record()
        changed["issuer_merkle_tree_ns"] = [0]
        with self.assertRaises(ValueError):
            validate_record(changed)


if __name__ == "__main__":
    unittest.main()
