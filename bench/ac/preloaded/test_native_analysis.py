"""[GPT-6] Sanitized native-output fixtures; no rows are experimental results."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('native_analysis_tested', Path(__file__).with_name('native_analysis.py'))
N = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(N)
from test_campaign_analysis import MEASUREMENT, records, request, write_rows


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + '\n')


def snapshot(sequence=0, memory=128):
    return {'errors': [], 'systemd': {'ControlGroup': '/sparq.slice/sparq-pod.slice/sparq-pod-bench.slice/native-fixture.service'},
        'cgroup': {'memory.current': '1500', 'memory.peak': '1600', 'memory.max': str(memory * N.GIB), 'memory.swap.max': '0',
                   'memory.events': 'oom 0\noom_kill 0\n',
                   'memory.stat': f'anon 800\nfile 700\npgfault {sequence * 10}\npgmajfault {sequence}\nworkingset_refault_file {sequence * 2}\n',
                   'cpu.stat': f'usage_usec {1000 + sequence * 100}\n', 'io.stat': f'259:0 rbytes={sequence * 4096} wbytes=0\n'},
        'process': {'status': 'Cpus_allowed_list:\t0-15\nVmRSS:\t2 kB\n', 'io': f'read_bytes: {sequence * 4096}\n',
                    'stat': '1 (fixture worker) S ' + '0 ' * 30, 'smaps_rollup': 'Rss: 2 kB\n', 'open_descriptors': 7},
        'host': {'meminfo': 'MemAvailable: 300000000 kB\n'}, 'study_slice': {'memory.current': '2000', 'memory.peak': '3000'}}


def worker(index):
    count = int(index == 0)
    return {'record_type': 'worker-population-ready', 'worker': index, 'assigned_pods': count, 'elapsed_us': 100,
        'state': {'storage_mode': 'native', 'population': 1, 'ready': True, 'poisoned': False,
                  'retained_pods': count, 'retained_source_bytes': count * 30,
                  'activations': {'rdf_parses': 0, 'native_dataset_loads': count, 'initial_authorizations': count,
                                  'journal_entries_replayed': 0, 'attempted_activations_after_ready': 0,
                                  'mutation_rollbacks': 0, 'rollback_authorizations': 0}}}


def seal(root, review):
    manifest = root / 'MANIFEST.sha256'
    manifest.write_text(''.join(f'{N.OLD.sha_file(path)}  ./{path.relative_to(root)}\n' for path in sorted(root.rglob('*')) if path.is_file() and path != manifest))
    review['manifest_sha256'] = N.OLD.sha_file(manifest)


def fixture(root, memory_tiers=(128,), repeats=2):
    source = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=N.ROOT, text=True).strip()
    config = {'volume_classes': [{'weight': 1, 'numerator': 1, 'denominator': 1}]}
    dataset = {'id': 'fixture-1', 'pods': 1, 'models': ['wac', 'acp'], 'role': 'synthetic-unit-fixture',
               'expected_config': config, 'expected_config_sha256': N.RUNNER.canonical_digest(config)}
    campaign = {'schema_version': 1, 'campaign_id': 'solid-pod-preloaded-unit-fixture', 'status': 'frozen-before-measurement',
        'source_commit': source, 'binary_build_source_commit': source, 'binary_sha256': 'b' * 64,
        'bindings': {path: N.OLD.sha_file(N.ROOT / path) for path in N.RUNNER.REQUIRED_BINDINGS},
        'corpora': [dataset], 'measurement': MEASUREMENT | {'queue_capacity_per_worker': 64}, 'seeds': [11, 22],
        'groups': [{'id': 'native', 'datasets': ['fixture-1'], 'memory_gib': list(memory_tiers), 'cpus': [16], 'rates': [4],
                    'repeat': repeats, 'scenario': 'busy-period', 'selection': 'uniform', 'storage_mode': 'native', 'max_pod_bytes': 100}],
        'execution': {'maximum_cells': len(memory_tiers) * repeats * 2},
        'preloaded': {'admission_outcomes_version': 1, 'startup_timeout_seconds': 100}, 'stop_rules': dict(N.RUNNER.STOP_BEHAVIOR),
        'host': {'instance_type': 'r7gd.12xlarge', 'physical_memory_gib': 384, 'physical_vcpus': 48, 'swap': False,
                 'server_cpu_pool': list(range(16)), 'client_cpu_pool': list(range(16, 20))}}
    write(root / 'frozen-campaign.json', campaign); campaign_hash = N.OLD.sha_file(root / 'frozen-campaign.json')
    source_hashes = N.source_bindings(campaign)['verified']
    completion = {'record_type': 'native-preloaded-campaign-result', 'status': 'complete', 'source_commit': source,
        'binary_build_source_commit': source, 'binary_sha256': 'b' * 64, 'campaign_sha256': campaign_hash,
        'source_quarantined': False, 'cleanup_errors': [], 'error': None, 'populations': [], 'cells': []}
    for model in ('wac', 'acp'):
        label = 'fixture-1-' + model
        manifest = {'format': 'sparq-pod-pack-zstd-v1', 'pods': 1, 'model': model, 'config': config,
            'records': 1, 'quads': 3, 'source_bytes': 30, 'packed_bytes': 10, 'index_bytes': 24,
            'maximum_pod_source_bytes': 30, 'populated': True, 'binary_payloads_included': False,
            'packed_sha256': 'c' * 64, 'index_sha256': 'd' * 64}
        write(root / (label + '-source-manifest.json'), manifest)
        source_manifest_sha = N.OLD.sha_file(root / (label + '-source-manifest.json'))
        native = {'format': 'sparq-population-native-v1', 'pods': 1, 'compressed': True,
                  'source_manifest_sha256': source_manifest_sha, 'source_bytes': 30, 'archive_bytes': 1000, 'archive_sha256': 'e' * 64}
        oracle = {'status': 'complete', 'resources': snapshot()}
        write(root / (label + '-oracle.json'), oracle)
        write_rows(root / (label + '-oracle.log'), [{'record_type': 'verify-pod-start', 'pod': 0},
            {'record_type': 'verify-progress', 'sampled_pods': 1, 'checks': 108},
            {'record_type': 'verification-complete', 'sampled_pods': 1, 'checks': 108}])
        write_rows(root / (label + '-pod-summaries.jsonl'), [{'pod_id': 0, 'records': 1, 'quads': 3, 'bytes': 30,
            'compressed_bytes': 10, 'records_by_service': {'communication': 1}, 'intensity_numerator': 1, 'intensity_denominator': 1}])
        population = {'dataset': 'fixture-1', 'model': model, 'status': 'complete', 'manifest': manifest,
            'source_manifest_sha256': source_manifest_sha, 'native_manifest': native, 'oracle': oracle,
            'generation': {'status': 'reused', 'new_preparation_time': None}, 'native_preparation': {'status': 'reused', 'new_preparation_time': None},
            'representatives': [{'pod': 0, 'source_bytes': 30, 'intensity_numerator': 1, 'intensity_denominator': 1}],
            'storage': {'entries': [{'path': path, 'kind': 'file', 'logical_bytes': size, 'allocated_bytes': 4096} for path, size in [('pods.nqpack', 10), ('pods.index', 24), ('pods.native', 1000)]],
                        'allocated_bytes_including_directories': 16384, 'files': 3, 'directories_including_root': 1, 'file_logical_bytes': 1034}}
        write(root / (label + '-population.json'), population); completion['populations'].append(population)
    for index, selection in enumerate(N.RUNNER.plan_cells(campaign)):
        directory = root / f'cell-{index:05d}'; directory.mkdir()
        label = f'fixture-1-native-ram{selection["memory_gib"]}-cpu16-r4.0-{selection["replicate"]}-{selection["model"]}'
        pop = next(row for row in completion['populations'] if row['model'] == selection['model']); native = pop['native_manifest']
        seed = campaign['seeds'][selection['replicate']]
        summary = {'dataset': 'fixture-1', 'group': 'native', 'memory_gib': selection['memory_gib'], 'cpus': 16, 'rate_override': 4.0,
                   'replicate': selection['replicate'], 'model': selection['model'], 'seed': seed, 'offered_rate': 4,
                   'load_exit_code': 0, 'warmup_exit_code': 0, 'audit_exit_code': 0}
        write(directory / (label + '-summary.json'), summary); write(directory / (label + '-reconciliation.json'), {'passed': True})
        raw = records([request(i, query_sha256='f' * 64) for i in range(4)])
        raw[0].update(settings={'seed': str(seed), 'scenario': 'busy-period', 'selection': 'uniform', 'mix': 'journeys',
            'timeout-ms': '5000', 'max-inflight': '8192', 'duration-seconds': '1', 'requests': '4', 'rate': '4.0'},
            workload_sha256=campaign['bindings']['bench/ac/million/workload.json'], corpus_manifest=pop['manifest'])
        write_rows(directory / (label + '-requests.jsonl'), raw)
        write_rows(directory / (label + '-audit.jsonl'), [{'record_type': 'mutation-audit-complete', 'committed_receipts': 0}])
        write(directory / 'preloaded-runtime.json', {'source_commit': source, 'binary_sha256': 'b' * 64, 'campaign_sha256': campaign_hash,
            'runner_sha256': source_hashes['bench/ac/preloaded/run-cell.py'], 'base_runner_sha256': source_hashes['bench/ac/million/run-campaign.py'],
            'workload_sha256': source_hashes['bench/ac/million/workload.json']})
        ready = {'record_type': 'all-population-ready', 'population': 1, 'retained_pods': 1,
            'worker_reports': [worker(i) for i in reversed(range(16))],
            'settings': {'storage-mode': 'native', 'workers': '16', 'max-pod-bytes': '100', 'queue-capacity': '64'},
            'archive_verification': {'record_type': 'native-archive-verified', 'manifest': native}}
        write_rows(directory / (label + '-server.log'), [ready['archive_verification'], ready, {'record_type': 'server-ready'}])
        write(directory / (label + '-startup-boundary.json'), {'complete': True, 'elapsed_seconds': 1,
            'cold_os_caches_before_launch': True, 'unit': 'native-fixture.service'})
        for phase, sequence in [('preload', 0), ('before', 1), ('after', 2)]:
            write(directory / (label + '-' + phase + '-native-accounting.json'), snapshot(sequence, selection['memory_gib']))
            if phase != 'preload':
                write(directory / (label + '-' + phase + '-drain.jsonl'), {'record_type': 'worker-drain-complete', 'passed': True,
                    'workers': [{'processed_requests': sequence * 4, 'state': worker(i)['state']} for i in range(16)]})
        completion['cells'].append({'selection': selection, 'status': 'complete', 'directory': directory.name, 'label': label, 'summary': summary})
    write(root / 'campaign-result.json', completion)
    review = {'status': 'passed', 'source_commit': source, 'binary_sha256': 'b' * 64, 'campaign_sha256': campaign_hash}
    seal(root, review)
    return review


class NativeAnalysisTests(unittest.TestCase):
    def analyze_fixture(self, root, review):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'review.json'; write(path, review)
            return N.analyze(root, path)

    def test_native_success_pairs_and_paging_stay_distinct_from_equivalence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); review = fixture(root)
            log = next(root.glob('cell-*/*-server.log'))
            subprocess.run(['zstd', '-q', '--rm', str(log)], check=True)
            seal(root, review)
            result = self.analyze_fixture(root, review)
            self.assertFalse(result['global_issues'], result['global_issues'])
            self.assertTrue(all(row['valid_for_inference'] for row in result['cells']), [row['issues'] for row in result['cells']])
            self.assertEqual(len(result['headline_eligible_cells']), 4)
            self.assertTrue(all(row['local_guard'] == 'pass' for row in result['cells']))
            pair = result['paired_comparisons'][0]
            self.assertTrue(pair['paired_p95_scheduled_response_ratio']['available'])
            self.assertEqual(pair['latency_equivalence'], 'not-assessed-no-frozen-margin')
            self.assertTrue(result['cells'][0]['resources']['paging']['positive_refault_and_major_fault_and_read_io'])

    def test_partial_campaign_or_stale_review_never_enters_headlines(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); review = fixture(root)
            result = self.analyze_fixture(root, review | {'binary_sha256': '0' * 64})
            self.assertFalse(result['headline_eligible_cells'])
            completion = json.loads((root / 'campaign-result.json').read_text()); completion['status'] = 'partial'; completion['cells'][-1]['status'] = 'unattempted'
            write(root / 'campaign-result.json', completion); seal(root, review)
            result = self.analyze_fixture(root, review)
            self.assertFalse(result['headline_eligible_cells'])
            self.assertEqual(result['cells'][-1]['local_guard'], 'unmeasured')
            self.assertEqual(result['cells'][0]['requests']['recorded'], 4)

    def test_reloaded_population_archive_mismatch_and_cpu_drift_are_rejected(self):
        for kind in ('activation', 'archive', 'cpu'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                root = Path(directory); review = fixture(root)
                if kind == 'activation':
                    path = next(root.glob('cell-*/*-after-drain.jsonl')); document = json.loads(path.read_text())
                    document['workers'][0]['state']['activations']['native_dataset_loads'] += 1
                elif kind == 'cpu':
                    path = next(root.glob('cell-*/*-before-native-accounting.json')); document = json.loads(path.read_text())
                    document['process']['status'] = 'Cpus_allowed_list: 0-3\n'
                else:
                    path = root / 'fixture-1-wac-source-manifest.json'; document = json.loads(path.read_text()); document['quads'] += 1
                write(path, document); seal(root, review)
                result = self.analyze_fixture(root, review)
                self.assertFalse(result['cells'][0]['valid_for_inference'])
                self.assertTrue(result['cells'][0]['issues'])

    def test_changed_query_breaks_pairing_without_erasing_valid_response_counts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); review = fixture(root)
            path = next(root.glob('cell-*/*-0-acp-requests.jsonl'))
            rows = [json.loads(line) for line in path.read_text().splitlines()]
            rows[1]['query_sha256'] = '0' * 64; write_rows(path, rows); seal(root, review)
            result = self.analyze_fixture(root, review)
            self.assertTrue(all(row['valid_for_inference'] for row in result['cells']))
            self.assertFalse(result['paired_comparisons'][0]['paired_p95_scheduled_response_ratio']['available'])

    def test_formal_equivalence_requires_the_actual_frozen_margin(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); review = fixture(root)
            campaign = json.loads((root / 'frozen-campaign.json').read_text())
            campaign['statistical_plan'] = {'practical_equivalence_margin_ratio': 1.1}
            write(root / 'frozen-campaign.json', campaign)
            digest = N.OLD.sha_file(root / 'frozen-campaign.json'); review['campaign_sha256'] = digest
            completion = json.loads((root / 'campaign-result.json').read_text()); completion['campaign_sha256'] = digest
            write(root / 'campaign-result.json', completion)
            for path in root.glob('cell-*/preloaded-runtime.json'):
                runtime = json.loads(path.read_text()); runtime['campaign_sha256'] = digest; write(path, runtime)
            seal(root, review)
            result = self.analyze_fixture(root, review)
            self.assertEqual(result['paired_comparisons'][0]['latency_equivalence'], 'within-margin')
            self.assertEqual(result['paired_comparisons'][0]['practical_equivalence_margin_ratio'], 1.1)
            self.assertTrue(result['sampled_authorization_correctness'][0]['both_sampled_neutral_oracles_passed'])

    def test_partial_request_stream_preserves_recorded_counts_without_inventing_offers(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); review = fixture(root)
            completion = json.loads((root / 'campaign-result.json').read_text())
            completion['status'] = 'partial'; completion['cells'][0]['status'] = 'partial'
            write(root / 'campaign-result.json', completion)
            path = next((root / 'cell-00000').glob('*-requests.jsonl'))
            rows = [json.loads(line) for line in path.read_text().splitlines()]
            write_rows(path, rows[:-1]); seal(root, review)
            result = self.analyze_fixture(root, review)
            self.assertFalse(result['headline_eligible_cells'])
            requests = result['cells'][0]['requests']
            self.assertEqual(requests['recorded'], 4)
            self.assertIsNone(requests['offered']); self.assertIsNone(requests['success_fraction_of_offered'])

    def test_compressed_canonical_quarantine_overrides_passing_summary_and_review(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); review = fixture(root)
            path = root / 'cell-00000/campaign-events.jsonl'
            write_rows(path, [{'record_type': 'correctness-quarantine', 'dataset': 'fixture-1', 'model': 'wac', 'reason': 'fixture independent oracle mismatch'}])
            subprocess.run(['zstd', '-q', '--rm', str(path)], check=True)
            seal(root, review)
            result = self.analyze_fixture(root, review)
            self.assertFalse(result['headline_eligible_cells'])
            self.assertEqual(result['source_review']['status'], 'quarantined')
            self.assertEqual(len(result['source_review']['quarantine_events']), 1)
            self.assertIn('canonical-source-quarantine', result['global_issues'])

    def test_reported_guard_disagreement_stays_visible_and_inconclusive(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); review = fixture(root)
            completion = json.loads((root / 'campaign-result.json').read_text())
            row = completion['cells'][0]; row['summary']['passes_local_guard'] = False
            write(root / row['directory'] / (row['label'] + '-summary.json'), row['summary'])
            write(root / 'campaign-result.json', completion); seal(root, review)
            result = self.analyze_fixture(root, review)
            self.assertFalse(result['cells'][0]['valid_for_inference'])
            self.assertEqual(result['cells'][0]['local_guard'], 'inconclusive')
            self.assertEqual(result['cells'][0]['reported_guard_disagreements'], [{'field': 'passes_local_guard', 'recorded': False, 'independent': True}])

    def test_failed_low_tier_is_negative_admission_only_and_higher_tier_remains_valid(self):
        for outcome in ('memory-limit', 'startup-timeout'):
            with self.subTest(outcome=outcome), tempfile.TemporaryDirectory() as directory:
                root = Path(directory); review = fixture(root, memory_tiers=(64, 128), repeats=1)
                self.replace_low_tier(root, outcome); seal(root, review)
                result = self.analyze_fixture(root, review)
                self.assertFalse(result['global_issues'], result['global_issues'])
                self.assertEqual(len(result['admission_failure_cells']), 2)
                self.assertEqual(len(result['headline_eligible_cells']), 2)
                for row in result['cells'][:2]:
                    self.assertTrue(row['valid_for_admission_inference'], row['issues'])
                    self.assertFalse(row['valid_for_inference']); self.assertIsNone(row['requests'])
                    self.assertEqual(row['local_guard'], 'admission-failed')
                self.assertTrue(all(row['valid_for_inference'] for row in result['cells'][2:]))
                self.assertFalse(result['paired_comparisons'][0]['paired_p95_scheduled_response_ratio']['available'])

    def test_unsubstantiated_admission_continuation_or_request_output_rejects_campaign(self):
        for corrupt in ('global-oom', 'request-output', 'bad-readiness', 'missing-capture'):
            with self.subTest(corrupt=corrupt), tempfile.TemporaryDirectory() as directory:
                root = Path(directory); review = fixture(root, memory_tiers=(64, 128), repeats=1)
                self.replace_low_tier(root, 'memory-limit', corrupt); seal(root, review)
                result = self.analyze_fixture(root, review)
                self.assertIn('unsupported-admission-continuation', result['global_issues'])
                self.assertEqual(result['headline_eligible_cells'], [])

    def replace_low_tier(self, root, outcome, corrupt=None):
        from test_preload_admission import admission
        completion = json.loads((root / 'campaign-result.json').read_text())
        for row in completion['cells'][:2]:
            directory = root / row['directory']; label = row['label']
            for path in directory.glob(label + '-*'): path.unlink()
            record = admission(outcome, memory=64)
            record['archive_manifest'] = next(p['native_manifest'] for p in completion['populations'] if p['model'] == row['selection']['model'])
            if corrupt == 'global-oom': record['terminal_capture']['cgroup']['memory.events.local'] = 'oom 0\noom_kill 1\n'
            rows = [{'record_type': 'native-archive-verified', 'manifest': record['archive_manifest']}]
            if corrupt == 'bad-readiness': rows.append({'record_type': 'all-population-ready', 'population': 0})
            log = directory / (label + '-server.log'); write_rows(log, rows)
            record['server_log_sha256'] = N.OLD.sha_file(log)
            write(directory / (label + '-preload-admission.json'), record)
            if corrupt != 'missing-capture': write(directory / (label + '-preload-terminal.json'), record['terminal_capture'])
            write(directory / (label + '-startup-boundary.json'), {'complete': False, 'elapsed_seconds': 102, 'cold_os_caches_before_launch': True, 'unit': record['unit']})
            write_rows(directory / 'campaign-events.jsonl', [{'record_type': 'preload-admission-failed', 'outcome': outcome, 'unit': record['unit'], 'population': 1, 'artifact': label + '-preload-admission.json'}])
            if corrupt == 'request-output': write_rows(directory / (label + '-requests.jsonl'), records([request(0)]))
            row.pop('summary'); row.update(status='admission-failed', admission=record)
        write(root / 'campaign-result.json', completion)


if __name__ == '__main__': unittest.main()
