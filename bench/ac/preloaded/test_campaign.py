"""[GPT-6] Frozen native-only orchestration, full inventory and partial evidence."""
import copy
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('native_campaign', Path(__file__).with_name('run-campaign.py'))
MAIN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MAIN)


def campaign():
    old = json.loads((MAIN.ROOT / 'bench/ac/million/campaign-20260906.json').read_text())
    shape = next(row for row in old['corpora'] if row['id'] == 'history-8')
    spec = {'schema_version': 1, 'status': 'frozen-before-measurement', 'campaign_id': 'solid-pod-preloaded-fixture',
        'source_commit': 'a' * 40, 'binary_build_source_commit': 'a' * 40, 'binary_sha256': 'b' * 64,
        'shipping_commits': ['a' * 40],
        'bindings': {path: MAIN.PREP.digest(MAIN.ROOT / path) for path in MAIN.REQUIRED_BINDINGS},
        'corpora': [shape | {'id': 'history-1000000', 'pods': 1000000, 'shape_reference': 'history-8',
                            'expected_config': {}, 'expected_config_sha256': MAIN.canonical_digest({})}],
        'groups': [{'id': 'native-fixture', 'datasets': ['history-1000000'], 'storage_mode': 'native',
                    'lane': 'journeys', 'cpus': [16], 'memory_gib': [64, 128], 'repeat': 2,
                    'scenario': 'busy-period', 'selection': 'uniform', 'max_pod_bytes': 2 * MAIN.PREP.GIB,
                    'cache_source_bytes': 2 * MAIN.PREP.GIB}],
        'host': {'instance_type': 'r7gd.12xlarge', 'physical_memory_gib': 384, 'physical_vcpus': 48,
                 'overall_memory_fraction': .8, 'swap': False, 'server_cpu_pool': list(range(16)), 'client_cpu_pool': list(range(16, 20))},
        'seeds': [11, 13], 'measurement': old['measurement'],
        'preloaded': {'version': 1, 'admission_outcomes_version': 1, 'fixed_server_cpus': 16, 'startup_timeout_seconds': 100,
                      'drain_timeout_seconds': 30, 'cell_timeout_seconds': 500},
        'execution': {'version': 1, 'data_mount': str(MAIN.PREP.DATA), 'data1_allocated': False,
                      'preparation_memory_gib': 128, 'phase_timeout_seconds': 200,
                      'inspection_timeout_seconds': 100, 'maximum_cells': 10},
        'stop_rules': MAIN.STOP_BEHAVIOR | {'runtime_ceiling_seconds': 3600,
                      'disk_floor_bytes': 100 * MAIN.PREP.GIB, 'result_disk_reserve_bytes': 2 * MAIN.PREP.GIB,
                      'result_maximum_bytes': 2 * MAIN.PREP.GIB, 'result_maximum_file_bytes': 512 * 1024**2}}
    return spec


class MainCampaignTests(unittest.TestCase):
    def test_frozen_larger_population_preserves_shape_and_paired_fixed_cpu_grid(self):
        spec = campaign(); rows = MAIN.validate_campaign(spec)
        self.assertEqual(len(rows), 8)
        self.assertEqual([row['model'] for row in rows[:4]], ['wac', 'acp', 'acp', 'wac'])
        self.assertEqual(set(row['rate'] for row in rows), {'derived'})
        self.assertEqual(spec['corpora'][0]['pods'], 1000000)
        for section, field, value in [('execution', 'data1_allocated', True),
                ('stop_rules', 'grid', 'skip-after-failure'), ('preloaded', 'fixed_server_cpus', 4),
                ('host', 'client_cpu_pool', list(range(4))), ('measurement', 'success_fraction', .5)]:
            wrong = copy.deepcopy(spec); wrong[section][field] = value
            with self.assertRaises(ValueError): MAIN.validate_campaign(wrong)
        for field, value in [('storage_mode', 'cached'), ('storage_mode', 'memory'), ('cpus', [4]), ('memory_gib', [128.0])]:
            wrong = copy.deepcopy(spec); wrong['groups'][0][field] = value
            with self.assertRaises(ValueError): MAIN.validate_campaign(wrong)
        wrong = copy.deepcopy(spec); wrong['status'] = 'proposal'
        with self.assertRaises(ValueError): MAIN.validate_campaign(wrong)
        wrong = copy.deepcopy(spec); wrong['corpora'][0]['profile'] = 'smoke'
        with self.assertRaises(ValueError): MAIN.validate_campaign(wrong)
        wrong = copy.deepcopy(spec); wrong['bindings']['crates/sparq-acbench/src/population.rs'] = '0' * 64
        with self.assertRaises(ValueError): MAIN.validate_campaign(wrong)
        for seeds in ([11, 11], [True, 13], [-1, 13], [2**64, 13], [11.0, 13]):
            wrong = copy.deepcopy(spec); wrong['seeds'] = seeds
            with self.assertRaises(ValueError): MAIN.validate_campaign(wrong)
        wrong = copy.deepcopy(spec); wrong['execution']['maximum_cells'] = 1
        with patch.object(MAIN, 'plan_cells', side_effect=AssertionError('must not allocate oversized plan')):
            with self.assertRaises(ValueError): MAIN.validate_campaign(wrong)
        wrong = copy.deepcopy(spec); wrong['host']['overall_memory_fraction'] = .1; wrong['groups'][0]['memory_gib'] = [1]
        with self.assertRaises(ValueError): MAIN.validate_campaign(wrong)
        for margin in (1, 0, True, '10%'):
            wrong = copy.deepcopy(spec); wrong['statistical_plan'] = {'practical_equivalence_margin_ratio': margin}
            with self.assertRaises(ValueError): MAIN.validate_campaign(wrong)

    def test_inventory_requires_every_actual_pod_index_and_unchanged_config(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'pods.nqpack').write_bytes(b'abcd')
            (root / 'pods.index').write_bytes(struct.pack('<QQQQQQ', 0, 2, 12, 2, 2, 14))
            rows = [{'pod_id': i, 'records': 1, 'quads': 4, 'bytes': size, 'compressed_bytes': 2,
                     'intensity_numerator': 1, 'intensity_denominator': 1} for i, size in enumerate((12, 14))]
            (root / 'pod-summaries.jsonl').write_text(''.join(json.dumps(row) + '\n' for row in rows))
            config = {'schema_version': 1, 'fixture': 'complete-index'}
            manifest = {'format': 'sparq-pod-pack-zstd-v1', 'populated': True, 'binary_payloads_included': False,
                'pods': 2, 'model': 'wac', 'config': config, 'records': 2, 'quads': 8,
                'source_bytes': 26, 'packed_bytes': 4, 'index_bytes': 48,
                'maximum_pod_source_bytes': 14, 'maximum_pod_compressed_bytes': 2,
                'packed_sha256': MAIN.PREP.digest(root / 'pods.nqpack'), 'index_sha256': MAIN.PREP.digest(root / 'pods.index')}
            (root / 'manifest.json').write_text(json.dumps(manifest))
            dataset = {'pods': 2, 'expected_config': config}
            self.assertEqual(MAIN.validate_population(root, dataset, 'wac')[0], manifest)
            with self.assertRaises(ValueError): MAIN.validate_population(root, dataset | {'pods': 1}, 'wac')
            with self.assertRaises(ValueError): MAIN.validate_population(root, dataset | {'expected_config': {}}, 'wac')
            (root / 'updates').mkdir()
            with self.assertRaises(ValueError): MAIN.validate_population(root, dataset, 'wac')
            (root / 'updates').rmdir()
            (root / 'pod-summaries.jsonl').write_text(json.dumps(rows[0]) + '\n')
            with self.assertRaises(ValueError): MAIN.validate_population(root, dataset, 'wac')
            (root / 'pod-summaries.jsonl').write_text(''.join(json.dumps(row) + '\n' for row in rows))
            (root / 'pods.index').write_bytes(struct.pack('<QQQQQQ', 0, 2, 12, 1, 2, 14))
            with self.assertRaises(ValueError): MAIN.validate_population(root, dataset, 'wac')

    def test_prepare_all_before_cells_and_preserve_failed_guard_without_skipping(self):
        self.run_fake(False)

    def test_preparation_failure_preserves_partial_and_all_unattempted_cells(self):
        self.run_fake(True)

    def test_cell_error_preserves_completed_partial_and_unattempted_rows(self):
        self.run_fake(False, cell_failure_at=3)

    def test_global_stop_between_cells_does_not_relabel_prior_completed_evidence(self):
        self.run_fake(False, stop_between_cells=True)

    def test_confirmed_low_memory_admission_failures_continue_to_higher_tier(self):
        self.run_fake(False, low_memory_failure=True)

    def run_fake(self, fail, cell_failure_at=None, stop_between_cells=False, low_memory_failure=False):
        trace = []; spec = campaign(); planned = MAIN.plan_cells(spec)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); auth = root / 'auth'; auth.mkdir()
            for name in ('client-secrets.json', 'issuer-public.json'): (auth / name).write_text('{}')
            args = SimpleNamespace(results=root / 'results', corpora=root / 'corpora', auth=auth, binary=root / 'binary',
                campaign=root / 'frozen.json', source_commit='a' * 40, binary_source_commit='a' * 40,
                binary_sha256='b' * 64, campaign_sha256='c' * 64)
            args.results.mkdir(); args.corpora.mkdir(); args.campaign.write_text(json.dumps(spec))
            runner = MAIN.Campaign(args, spec, planned)
            def guard(estimate=0):
                if stop_between_cells and trace.count('cell') == 1: raise RuntimeError('fixture global disk stop')
            runner.guard = guard
            runner.verify_inputs = lambda: None
            runner.preparation = SimpleNamespace(require_exclusive_jobs=lambda: None, stop=lambda: trace.append('cleanup'))
            def prepare(dataset, model):
                trace.append('prepare-' + model)
                runner.populations.append({'dataset': dataset['id'], 'model': model, 'status': 'partial' if fail else 'complete'})
                (args.corpora / (dataset['id'] + '-' + model)).mkdir()
                if fail: raise RuntimeError('fixture preparation failed')
            runner.prepare = prepare
            class FakeCell:
                def __init__(self, owner, selection, result_dir):
                    result_dir.mkdir()
                    self.memory = selection['memory_gib']
                    self.selection = (spec['corpora'][0], spec['groups'][0], selection['rate'], str(len(trace)))
                def cell(self, *args):
                    trace.append('cell')
                    if cell_failure_at == trace.count('cell'): raise RuntimeError('fixture cell failed after partial output')
                    if low_memory_failure and self.memory == 64:
                        from test_preload_admission import admission
                        raise MAIN.CELL.ADMISSION.AdmissionFailure(admission('memory-limit', memory=64))
                    return {'correctness_passed': True, 'passes_local_guard': False}
                def stop_server(self): pass
            with patch.object(MAIN, 'MainCell', FakeCell):
                if fail or cell_failure_at or stop_between_cells:
                    with self.assertRaises(RuntimeError): runner.execute()
                else: runner.execute()
            result = json.loads((args.results / 'campaign-result.json').read_text())
            self.assertTrue((args.results / 'MANIFEST.sha256').exists())
            if fail:
                self.assertEqual(result['status'], 'partial')
                self.assertEqual(set(row['status'] for row in result['cells']), {'unattempted'})
                self.assertEqual(result['unattempted_populations'], [{'dataset': 'history-1000000', 'model': 'acp'}])
                self.assertNotIn('cell', trace)
            elif stop_between_cells:
                self.assertEqual(result['status'], 'partial')
                self.assertEqual([row['status'] for row in result['cells']], ['complete'] + ['unattempted'] * 7)
                self.assertNotIn('error', result['cells'][0])
                self.assertIn('fixture global disk stop', result['error'])
            elif cell_failure_at:
                self.assertEqual(result['status'], 'partial')
                self.assertEqual([row['status'] for row in result['cells']], ['complete', 'complete', 'partial'] + ['unattempted'] * 5)
                self.assertIn('fixture cell failed', result['cells'][2]['error'])
                self.assertFalse(result['source_quarantined'])
            elif low_memory_failure:
                self.assertEqual(result['status'], 'complete')
                self.assertEqual([row['status'] for row in result['cells']], ['admission-failed'] * 4 + ['complete'] * 4)
                self.assertTrue(all('summary' not in row and row['admission']['timed_load_started'] is False for row in result['cells'][:4]))
                self.assertEqual(trace.count('cell'), 8)
            else:
                self.assertEqual(trace[:3], ['prepare-wac', 'prepare-acp', 'cell'])
                self.assertEqual(result['status'], 'complete')
                self.assertEqual(sum(row['status'] == 'complete' for row in result['cells']), 8)
                self.assertTrue(all(not row['summary']['passes_local_guard'] for row in result['cells']))


if __name__ == '__main__': unittest.main()
