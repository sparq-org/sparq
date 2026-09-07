"""[GPT-6] Source identity and resource admission for preparation-only execution."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('preparation', Path(__file__).with_name('run-preparation.py'))
PREP = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREP)


def resources():
    return {'errors': [], 'cgroup': {'memory.max': str(128 * PREP.GIB), 'memory.swap.max': '0',
        'memory.peak': '1000', 'memory.events': 'oom 0\noom_kill 0\n',
        'memory.stat': 'anon 500\nfile 500\n', 'cpu.stat': 'usage_usec 10\n'}}


class PreparationTests(unittest.TestCase):
    def test_auth_accepts_empty_directory_but_preserves_partial_keys(self):
        with tempfile.TemporaryDirectory() as directory:
            auth = Path(directory) / 'auth'
            self.assertTrue(PREP.auth_requires_provision(auth))
            auth.mkdir()
            self.assertTrue(PREP.auth_requires_provision(auth))
            (auth / 'client-secrets.json').write_text('{}')
            with self.assertRaises(ValueError): PREP.auth_requires_provision(auth)
            (auth / 'issuer-public.json').write_text('{}')
            self.assertFalse(PREP.auth_requires_provision(auth))

    def test_inspection_deadline_interrupts_blocking_read(self):
        read, write = os.pipe()
        try:
            with self.assertRaises(TimeoutError):
                with PREP.bounded_inspection(.02): os.read(read, 1)
        finally: os.close(read); os.close(write)

    def test_history_projection_requires_heavy_prefix_and_never_admits_population(self):
        proposal = {'cells': [{'dataset': dataset, 'model': model, 'prior_observations': {'pods': pods, 'records': pods * 100}}
                    for dataset, pods in [('history-8', 8), ('history-64', 64)] for model in ('wac', 'acp')]}
        def completed(dataset, pods):
            return {'dataset': dataset, 'model': 'wac', 'status': 'complete',
                    'storage': {'entries': [{'path': 'pods.native', 'logical_bytes': pods * 200}]},
                    'after_drain_resources': {'cgroup': {'memory.stat': f'anon {pods * 100}\nfile 1000\n'}},
                    'startup': {'elapsed_seconds_including_unit_start_and_observation': pods * .1},
                    'preparation': {'elapsed_seconds_including_unit_start_and_observation': pods * .2}}
        rows = [completed('history-8', 8)]
        self.assertFalse(PREP.history_projections(proposal, rows, [])['models'][0]['available'])
        rows.append(completed('history-64', 64))
        result = PREP.history_projections(proposal, rows, [])
        self.assertEqual([row['pods'] for row in result['models'][0]['targets']], [1000, 10000, 100000, 1000000])
        self.assertEqual(result['models'][0]['targets'][-1]['ranges']['native_bytes'], [200000000, 200000000])
        self.assertTrue(all(row['status'] == 'modeled-unmeasured-native-population' for row in result['models'][0]['targets']))
        self.assertFalse(result['models'][1]['available'])

    def test_binary_source_equivalence_rejects_any_unlisted_input_including_rules(self):
        with patch.object(PREP.subprocess, 'check_output', return_value=b'bench/ac/preloaded/run-preparation.py\0'):
            self.assertTrue(PREP.source_equivalence('a' * 40, 'b' * 40)['passed'])
        for path in ('crates/sparq-solid/rules/acp.n3', 'Cargo.lock', 'config/embedded.json', 'crates/sparq-core/src/lib.rs'):
            with patch.object(PREP.subprocess, 'check_output', return_value=path.encode() + b'\0'):
                with self.assertRaises(ValueError): PREP.source_equivalence('a' * 40, 'b' * 40)

    def test_oom_missing_accounting_wrong_cap_or_swap_prevents_admission(self):
        PREP.validate_resources(resources())
        for key, value in [('memory.events', 'oom 1\noom_kill 0\n'), ('memory.stat', 'anon 500\n'),
                           ('memory.max', str(256 * PREP.GIB)), ('memory.swap.max', '1'),
                           ('memory.peak', str(128 * PREP.GIB + 1)), ('cpu.stat', 'unknown 0')]:
            row = resources(); row['cgroup'][key] = value
            with self.assertRaises(ValueError): PREP.validate_resources(row)
        row = resources(); row['errors'] = ['process status absent']
        with self.assertRaises(ValueError): PREP.validate_resources(row)

    def test_regenerated_complete_payload_and_extrema_must_match(self):
        with tempfile.TemporaryDirectory() as directory:
            corpus = Path(directory)
            (corpus / 'pods.nqpack').write_bytes(b'fixture-pack')
            (corpus / 'pods.index').write_bytes(bytes(24))
            summary = {'pod_id': 0, 'bytes': 12, 'intensity_numerator': 1, 'intensity_denominator': 1}
            (corpus / 'pod-summaries.jsonl').write_text(json.dumps(summary) + '\n')
            prior = {'pods': 1, 'source_bytes': 12, 'packed_bytes': 12, 'index_bytes': 24,
                     'packed_sha256': PREP.digest(corpus / 'pods.nqpack'), 'index_sha256': PREP.digest(corpus / 'pods.index')}
            manifest = prior | {'model': 'wac', 'populated': True, 'format': 'sparq-pod-pack-zstd-v1'}
            (corpus / 'manifest.json').write_text(json.dumps(manifest))
            cell = {'model': 'wac', 'prior_observations': prior, 'existing_intensity_extrema':
                    [{'pod': 0, 'source_bytes': 12, 'intensity_numerator': 1, 'intensity_denominator': 1}]}
            self.assertEqual(PREP.validate_corpus(corpus, cell), manifest)
            wrong = copy.deepcopy(cell); wrong['existing_intensity_extrema'][0]['source_bytes'] = 13
            with self.assertRaises(ValueError): PREP.validate_corpus(corpus, wrong)
            (corpus / 'updates').mkdir(); (corpus / 'updates/0.jsonl').write_text('{}\n')
            with self.assertRaises(ValueError): PREP.validate_corpus(corpus, cell)
            (corpus / 'updates/0.jsonl').unlink()
            (corpus / 'pods.nqpack').write_bytes(b'changed-pack')
            with self.assertRaises(ValueError): PREP.validate_corpus(corpus, cell)

    def test_source_binding_and_complete_paired_definition_are_required(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / 'bench/ac/million').mkdir(parents=True); (root / 'research').mkdir()
            definitions = [{'id': str(i), 'pods': 1, 'profile': 'smoke', 'models': ['wac', 'acp']} for i in range(6)]
            prior = {'pods': 1, 'source_bytes': 12}
            rows = [{'dataset': d['id'], 'model': m, 'storage_inventory_consistent': True,
                     'manifest': prior, 'verification': {'expected_representatives': []}}
                    for d in definitions for m in ('wac', 'acp')]
            inputs = {'bench/ac/million/campaign-20260906.json': {'corpora': definitions},
                      'research/solid-pod-scale-main.json': {'corpora': rows}}
            for path, value in inputs.items(): (root / path).write_text(json.dumps(value))
            proposal = {'schema_version': 1, 'status': 'prospective footprint proposal only; fixture',
                'bindings': {path: PREP.digest(root / path) for path in inputs},
                'cells': [{'dataset': d['id'], 'model': m, 'definition': d, 'prior_observations': prior,
                           'existing_intensity_extrema': []} for d in definitions for m in ('wac', 'acp')],
                'proposed_environment': {'server_cpus': 16, 'disjoint_client_cpus': 4, 'preparation_and_reference_startup_memory_gib': 128},
                'proposed_bounds': {'per_command_timeout_seconds': 900, 'pilot_total_wall_seconds': 3600}}
            PREP.validate_proposal(proposal, root)
            wrong = copy.deepcopy(proposal); wrong['cells'].pop()
            with self.assertRaises(ValueError): PREP.validate_proposal(wrong, root)
            wrong = copy.deepcopy(proposal); wrong['cells'][0]['definition']['pods'] = 0
            with self.assertRaises(ValueError): PREP.validate_proposal(wrong, root)
            wrong = copy.deepcopy(proposal); wrong['cells'][0]['prior_observations']['source_bytes'] = 1
            with self.assertRaises(ValueError): PREP.validate_proposal(wrong, root)
            (root / 'research/solid-pod-scale-main.json').write_text('{}')
            with self.assertRaises(ValueError): PREP.validate_proposal(proposal, root)

    def test_finished_phase_retains_accounting_then_stops_owned_unit(self):
        class Fake(PREP.Preparation):
            def guard(self): pass
            def launch(self, argv, label):
                self.current = 'unit'; return 'unit', self.results / 'unused.log', time.monotonic() + 10
            def properties(self, unit): return {'SubState': 'exited', 'ActiveState': 'active', 'Result': 'success', 'ExecMainStatus': '0'}
            def snapshot(self, unit): return resources()
            def stop(self): self.current = None
        with tempfile.TemporaryDirectory() as directory:
            pilot = Fake(SimpleNamespace(results=Path(directory)), {'proposed_bounds': {'pilot_total_wall_seconds': 3600, 'per_command_timeout_seconds': 900}})
            report = pilot.phase(['binary', 'prepare-native'], 'prepare')
            self.assertEqual(report['status'], 'complete'); self.assertIsNone(pilot.current)
            self.assertEqual(json.loads((Path(directory) / 'prepare.json').read_text())['resources'], resources())
            pilot.snapshot = lambda _unit: {'errors': ['resource capture missing']}
            with self.assertRaises(ValueError): pilot.phase(['binary', 'prepare-native'], 'failed')
            self.assertEqual(json.loads((Path(directory) / 'failed.json').read_text())['status'], 'incomplete')
            self.assertIsNone(pilot.current)


if __name__ == '__main__': unittest.main()
