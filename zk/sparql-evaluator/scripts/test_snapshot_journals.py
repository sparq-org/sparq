"""Behavioural checks of the native full-journal comparison boundary."""

from copy import deepcopy
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from compare_snapshot_journals import ADDED, CAPACITY_SOURCE, RESTORED, VERSIONS, compare, load


class SnapshotJournalComparison(unittest.TestCase):
    def fixtures(self):
        before, after = {}, {}
        for case in RESTORED:
            for holder in (False, True):
                key = (CAPACITY_SOURCE, case, holder)
                before[key] = {version: {"journal": {"result": True}} for version in VERSIONS}
                after[key] = {
                    version: {"error": "Capacity(NumericRepresentation)"} for version in VERSIONS
                }
        key = ("control.json", "accepted", False)
        before[key] = {version: {"journal": {"result": [None, "value"]}} for version in VERSIONS}
        after[key] = deepcopy(before[key])
        for case in ADDED:
            for holder in (False, True):
                after[(CAPACITY_SOURCE, case, holder)] = {
                    version: {"error": "Capacity(NumericRepresentation)"} for version in VERSIONS
                }
        return before, after

    def test_reviewed_rejections_preserve_all_control_journals(self):
        before, after = self.fixtures()
        self.assertEqual(compare(before, after)["preserved_success_journals"], len(VERSIONS))

    def test_changed_success_journal_is_rejected(self):
        before, after = self.fixtures()
        after[("control.json", "accepted", False)]["v1"]["journal"]["result"] = []
        with self.assertRaisesRegex(ValueError, "unexpected journal/outcome change"):
            compare(before, after)

    def test_missing_restored_bound_or_wrong_cause_is_rejected(self):
        for cause in ("missing", "Capacity(TemporalYear)"):
            before, after = self.fixtures()
            key = next(iter(before))
            after[key]["v1"] = before[key]["v1"] if cause == "missing" else {"error": cause}
            with self.assertRaises(ValueError):
                compare(before, after)

    def test_missing_corpus_record_is_rejected(self):
        before, after = self.fixtures()
        del after[next(iter(after))]
        with self.assertRaisesRegex(ValueError, "inventories differ"):
            compare(before, after)

    def test_each_added_case_requires_both_authorities_and_every_version(self):
        for case in ADDED:
            for holder in (False, True):
                for version in VERSIONS:
                    before, after = self.fixtures()
                    after[(CAPACITY_SOURCE, case, holder)][version] = {"journal": {"result": []}}
                    with self.assertRaisesRegex(ValueError, "unexpected added capacity outcome"):
                        compare(before, after)
                before, after = self.fixtures()
                del after[(CAPACITY_SOURCE, case, holder)]
                with self.assertRaisesRegex(ValueError, "inventories differ"):
                    compare(before, after)

    def test_unreviewed_addition_is_rejected(self):
        before, after = self.fixtures()
        after[(CAPACITY_SOURCE, "other", False)] = after[(CAPACITY_SOURCE, next(iter(ADDED)), False)]
        with self.assertRaisesRegex(ValueError, "inventories differ"):
            compare(before, after)

    def test_empty_and_duplicate_outputs_are_rejected(self):
        with TemporaryDirectory() as directory:
            path = Path(directory) / "records.jsonl"
            path.write_text("")
            with self.assertRaisesRegex(ValueError, "empty corpus"):
                load(path)
            record = '{"source":"c", "id":"i", "holder":false}\n'
            path.write_text(record + record)
            with self.assertRaisesRegex(ValueError, "duplicate record"):
                load(path)


if __name__ == "__main__":
    unittest.main()
