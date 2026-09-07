"""[GPT-6] Native paper adapter checks using temporary, explicitly synthetic evidence."""
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import test_native_analysis as NTEST
from test_native_analysis import N, fixture, seal, write

ROOT = Path(__file__).resolve().parents[3]
TYPST = os.environ.get('TYPST_BIN') or shutil.which('typst')


@unittest.skipUnless(TYPST, 'set TYPST_BIN for native presentation checks')
class NativePaperBindings(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='native-paper-fixture-', dir=ROOT)
        self.addCleanup(self.temp.cleanup); self.directory = Path(self.temp.name)
        self.analysis = self.make_analysis('baseline')

    def make_analysis(self, name, admission=False):
        artifacts = self.directory / name; artifacts.mkdir()
        review = fixture(artifacts, memory_tiers=(64, 128) if admission else (128,), repeats=1 if admission else 2)
        if admission: NTEST.NativeAnalysisTests().replace_low_tier(artifacts, 'memory-limit'); seal(artifacts, review)
        path = self.directory / (name + '-review.json'); write(path, review)
        return N.analyze(artifacts, path)

    def query(self, body, data=None):
        if data is not None: write(self.directory / 'analysis.json', data)
        source = self.directory / 'binding.typ'
        source.write_text('#import "../site/papers/solid-pod-preloaded-results.typ": native-result-state, native-response-rows, native-response-table, native-history-admission\n' + body)
        result = subprocess.run([TYPST, 'query', '--root', str(ROOT), str(source), '<result>', '--field', 'value'],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_native_schema_opens_without_loading_any_default_result(self):
        self.assertEqual(self.query('#metadata(native-result-state(none)) <result>'), ['unmeasured'])
        result = self.query('''#let data = json("analysis.json")
#native-response-table(data)
#metadata((native-result-state(data), native-response-rows(data).len(), native-history-admission(data).by_model.wac.state)) <result>
''', self.analysis)
        self.assertEqual(result, [['finalized and reviewed', 4, 'unmeasured']])

    def test_old_partial_quarantined_and_mismatched_inputs_do_not_render_rows(self):
        variants = [
            (('analysis_kind',), 'frozen-campaign-independent-accounting'),
            (('execution_status',), 'partial'),
            (('source_review', 'status'), 'quarantined'),
            (('source_review', 'quarantine_events'), [{'record_type': 'correctness-quarantine'}]),
            (('events',), [{'record_type': 'correctness-quarantine'}]),
            (('artifact_integrity', 'complete'), False),
            (('artifact_integrity', 'parsing_errors'), [{'error': 'broken'}]),
            (('source_input_verification', 'passed'), False),
            (('global_issues',), ['unsupported-admission-continuation']),
            (('source_review', 'matches_exact_result'), False),
            (('source_review', 'record', 'manifest_sha256'), '0' * 64),
            (('source_review', 'record', 'source_commit'), '0' * 40),
            (('source_review', 'record', 'binary_sha256'), '0' * 64),
            (('source_review', 'record', 'campaign_sha256'), '0' * 64),
            (('binary_build_source_commit',), '0' * 40),
        ]
        for path, value in variants:
            with self.subTest(path=path):
                data = copy.deepcopy(self.analysis); target = data
                for key in path[:-1]: target = target[key]
                target[path[-1]] = value
                result = self.query('#let data = json("analysis.json")\n#metadata((native-result-state(data), native-response-rows(data).len(), native-history-admission(data))) <result>', data)[0]
                self.assertNotEqual(result[0], 'finalized and reviewed')
                self.assertEqual(result[1:], [0, None])

    def test_real_adapter_shape_keeps_negative_admission_latency_null(self):
        data = self.make_analysis('negative-admission', admission=True)
        result = self.query('''#let data = json("analysis.json")
#native-response-table(data)
#metadata((native-result-state(data), native-response-rows(data))) <result>
''', data)[0]
        self.assertEqual(result[0], 'finalized and reviewed')
        for row in result[1][:2]:
            self.assertEqual(row['state'], 'admission-failed')
            self.assertIsNone(row['successful_p95_us'])
            self.assertIsNone(row['success_fraction_of_offered'])
            self.assertIsNone(row['deadline_fraction_of_offered'])
        self.assertTrue(all(row['successful_p95_us'] is not None for row in result[1][2:]))

    def test_successful_latency_does_not_fall_back_to_fast_errors(self):
        data = copy.deepcopy(self.analysis)
        cell = data['cells'][0]
        cell['requests']['latency_us']['successful:scheduled_latency_us']['p95'] = None
        cell['requests']['latency_us']['all:scheduled_latency_us'] = {'p95': 1}
        selected = self.query('#metadata(native-response-rows(json("analysis.json"), labels: ("' + cell['label'] + '",))) <result>', data)[0]
        self.assertEqual(len(selected), 1); self.assertIsNone(selected[0]['successful_p95_us'])


if __name__ == '__main__': unittest.main()
