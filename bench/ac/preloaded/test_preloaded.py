"""[GPT-6] Admission checks for complete preload and worker drain records."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest
import tempfile
from types import SimpleNamespace
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('preloaded_cell', Path(__file__).with_name('run-cell.py'))
CELL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CELL)


def barrier(mode='native'):
    return {'record_type': 'worker-drain-complete', 'passed': True, 'workers': [
        {'processed_requests': 10, 'state': {'storage_mode': mode, 'population': 3,
            'ready': True, 'poisoned': False, 'retained_pods': count,
            'activations': {'rdf_parses': count if mode == 'memory' else 0,
                'native_dataset_loads': count if mode == 'native' else 0,
                'initial_authorizations': count, 'journal_entries_replayed': 0,
                'attempted_activations_after_ready': 0, 'mutation_rollbacks': 0,
                'rollback_authorizations': 0}}} for count in (2, 1)]}


class PreloadAdmissionTests(unittest.TestCase):
    def test_live_split_readiness_write_waits_for_complete_line(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); corpus = root / 'corpus'; corpus.mkdir()
            (corpus / 'manifest.json').write_text('{"pods": 1}')
            (corpus / 'pods.native-manifest.json').write_text('{}')
            cell = CELL.PreloadedCell.__new__(CELL.PreloadedCell)
            cell.preloaded = {'fixed_server_cpus': 16, 'admission_outcomes_version': 1, 'startup_timeout_seconds': 5}
            cell.spec = {'host': {'server_cpu_pool': list(range(16))}}
            cell.results = root; cell.binary = root / 'binary'; cell.auth = root; cell.control_file = root / 'token'
            cell.measurement = {'queue_capacity_per_worker': 64}; cell.counter = 0
            cell.guard = lambda: None; captured = []; cell.resource = lambda unit, label: captured.append(label)
            log = root / 'fixture-server.log'
            ready = json.dumps({'record_type': 'all-population-ready', 'population': 1, 'retained_pods': 1})
            def run(command, **kwargs):
                if 'systemd-run' in command: log.write_text(ready[:20])
                return SimpleNamespace(stdout='active\n', returncode=0)
            def finish_write(_seconds):
                with log.open('a') as stream: stream.write(ready[20:] + '\n{"record_type":"server-ready"}\n')
            with patch.object(CELL.subprocess, 'run', side_effect=run), patch.object(CELL.time, 'sleep', side_effect=finish_write):
                unit = cell.start_server(corpus, {'storage_mode': 'native', 'max_pod_bytes': 100}, 64, 16, 'fixture')
            self.assertTrue(unit.startswith('sparq-pod-preloaded-'))
            self.assertEqual(captured, ['fixture-preload'])

    def test_cell_identity_and_fixed_resource_admission_precede_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            args = SimpleNamespace(dataset='compact', group='native', model='wac', memory_gib=128,
                replicate=0, rate='derived', results=Path(directory) / 'results')
            spec = {'campaign_id': 'solid-pod-preloaded-test', 'preloaded': {'version': 1, 'fixed_server_cpus': 16},
                'host': {'server_cpu_pool': list(range(16)), 'client_cpu_pool': list(range(16, 20)),
                    'physical_vcpus': 48, 'physical_memory_gib': 384, 'overall_memory_fraction': .8, 'swap': False},
                'corpora': [{'id': 'compact', 'models': ['wac', 'acp']}], 'seeds': [11],
                'groups': [{'id': 'native', 'datasets': ['compact'], 'memory_gib': [128],
                    'repeat': 1, 'cpus': [16], 'storage_mode': 'native'}]}
            self.assertEqual(CELL.selected_cell(args, spec)[2], 'derived')
            self.assertFalse(args.results.exists())
            for section, key, value in [('host', 'client_cpu_pool', [0, 17, 18, 19]),
                    ('host', 'swap', True), ('host', 'physical_memory_gib', 128),
                    ('preloaded', 'fixed_server_cpus', 4)]:
                wrong = copy.deepcopy(spec); wrong[section][key] = value
                with self.assertRaises(ValueError): CELL.selected_cell(args, wrong)
            wrong = copy.deepcopy(spec); wrong['campaign_id'] = 'solid-pod-scale-20260906-main'
            with self.assertRaises(ValueError): CELL.selected_cell(args, wrong)
            args.results.mkdir(); (args.results / 'evidence').write_text('retain')
            with self.assertRaises(ValueError): CELL.selected_cell(args, spec)
            self.assertEqual((args.results / 'evidence').read_text(), 'retain')

    def test_complete_native_and_memory_partitions_are_admitted(self):
        for mode in ('native', 'memory'):
            before = barrier(mode)
            after = copy.deepcopy(before)
            after['workers'][0]['processed_requests'] += 20
            # Actual failed-mutation recovery is not a read-time activation.
            after['workers'][0]['state']['activations']['mutation_rollbacks'] += 1
            after['workers'][0]['state']['activations']['rollback_authorizations'] += 1
            CELL.validate_barrier(before, 3, 2, mode)
            CELL.validate_barrier(after, 3, 2, mode, before)

    def test_omitted_or_duplicate_population_and_missing_fence_fail(self):
        for retained in (0, 2):
            record = barrier()
            record['workers'][1]['state']['retained_pods'] = retained
            with self.assertRaises(ValueError): CELL.validate_barrier(record, 3, 2, 'native')
        record = barrier(); record['workers'].pop()
        with self.assertRaises(ValueError): CELL.validate_barrier(record, 3, 2, 'native')

    def test_late_load_parse_replay_or_missing_auth_is_rejected(self):
        for key in ('rdf_parses', 'native_dataset_loads', 'initial_authorizations', 'journal_entries_replayed', 'attempted_activations_after_ready'):
            before = barrier(); after = copy.deepcopy(before)
            after['workers'][0]['state']['activations'][key] += 1
            with self.assertRaises(ValueError): CELL.validate_barrier(after, 3, 2, 'native', before)

    def test_poison_wrong_mode_false_completion_or_counter_regression_fail(self):
        for field, value in (('poisoned', True), ('ready', False), ('population', 4), ('storage_mode', 'cached')):
            record = barrier(); record['workers'][0]['state'][field] = value
            with self.assertRaises(ValueError): CELL.validate_barrier(record, 3, 2, 'native')
        record = barrier(); record['passed'] = False
        with self.assertRaises(ValueError): CELL.validate_barrier(record, 3, 2, 'native')
        before = barrier(); after = copy.deepcopy(before); after['workers'][0]['processed_requests'] -= 1
        with self.assertRaises(ValueError): CELL.validate_barrier(after, 3, 2, 'native', before)


if __name__ == '__main__':
    unittest.main()
