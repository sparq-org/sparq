#!/usr/bin/env python3
"""[GPT-6] Prepare every frozen population, then execute only preloaded cells."""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import importlib.util
import itertools
import json
import math
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import struct
import subprocess
import time
from types import SimpleNamespace

SPEC = importlib.util.spec_from_file_location('native_preparation', Path(__file__).with_name('run-preparation.py'))
PREP = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREP)
CELL = PREP.CELL
BASE = CELL.BASE
ROOT = PREP.ROOT
REQUIRED_BINDINGS = {
    'bench/ac/preloaded/protocol.json', 'bench/ac/million/campaign-20260906.json',
    'bench/ac/million/workload.json', 'bench/ac/million/corpus-calibration.json',
    'crates/sparq-acbench/src/population.rs', 'crates/sparq-acbench/src/population_ratings_cdf.json',
    'bench/ac/preloaded/preload_admission.py',
}
STOP_BEHAVIOR = {
    'grid': 'complete-declared-grid',
    'preparation_failure': 'stop-before-all-timed-cells',
    'execution_error': 'stop-and-preserve-partial',
    'correctness_failure': 'quarantine-source-and-stop',
    'observed_guard_failure': 'continue-declared-grid',
    'preload_admission_failure': 'continue-only-confirmed-local-oom-or-startup-timeout',
}


def positive(value, name):
    if type(value) not in (int, float) or not math.isfinite(value) or value <= 0:
        raise ValueError(f'{name} must be positive and finite')
    return value


def canonical_digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def plan_cells(spec):
    cells = []
    for dataset in spec['corpora']:
        for group in spec['groups']:
            if dataset['id'] not in group['datasets']: continue
            for memory, rate, replicate in itertools.product(group['memory_gib'], group.get('rates', ['derived']), range(group['repeat'])):
                for model in (('wac', 'acp') if replicate % 2 == 0 else ('acp', 'wac')):
                    cells.append({'dataset': dataset['id'], 'group': group['id'], 'memory_gib': memory,
                                  'rate': rate, 'replicate': replicate, 'model': model})
    return cells


def validate_campaign(spec, root=ROOT):
    """Only the prospective document chooses populations/rates; never infer a grid."""
    if spec.get('schema_version') != 1 or spec.get('status') != 'frozen-before-measurement' or not spec.get('campaign_id', '').startswith('solid-pod-preloaded-'):
        raise ValueError('require a separately named frozen preloaded campaign')
    if spec['execution']['version'] != 1 or spec['execution']['data_mount'] != str(PREP.DATA) or spec['execution']['data1_allocated'] is not False:
        raise ValueError('unknown executor version or data allocation')
    if spec['preloaded'].get('admission_outcomes_version') != 1:
        raise ValueError('explicit typed preload admission outcomes required')
    for key, expected in STOP_BEHAVIOR.items():
        if spec['stop_rules'].get(key) != expected: raise ValueError(f'unsupported or missing frozen stop rule: {key}')
    if not REQUIRED_BINDINGS <= spec['bindings'].keys(): raise ValueError('required generator/workload/protocol source binding missing')
    for relative, expected in spec['bindings'].items():
        if PREP.digest(PREP.within(root / relative, root)) != expected:
            raise ValueError(f'frozen input drift: {relative}')
    old = json.loads((root / 'bench/ac/million/campaign-20260906.json').read_text())
    definitions = {row['id']: row for row in old['corpora']}
    datasets = set()
    for dataset in spec['corpora']:
        if not re.fullmatch('[a-zA-Z0-9._-]+', dataset['id']) or dataset['id'] in datasets: raise ValueError('invalid or duplicate dataset id')
        datasets.add(dataset['id'])
        if type(dataset['pods']) is not int or dataset['pods'] <= 0: raise ValueError('complete positive population required')
        reference = definitions[dataset['shape_reference']]
        for key in ('profile', 'role', 'models', 'config_file'):
            if dataset.get(key) != reference.get(key): raise ValueError('frozen corpus changes an existing service-history shape')
        if dataset['models'] != ['wac', 'acp']: raise ValueError('both policy models required for every population')
        if canonical_digest(dataset['expected_config']) != dataset['expected_config_sha256']: raise ValueError('frozen full generator configuration digest differs')
        if dataset.get('config_file'):
            if dataset['config_file'] not in spec['bindings'] or json.loads((root / dataset['config_file']).read_text()) != dataset['expected_config']:
                raise ValueError('configured history shape differs from bound source')
    if not datasets or not spec['groups']: raise ValueError('empty campaign is not a measurement matrix')
    seeds = spec['seeds']
    if not seeds or any(type(seed) is not int or not 0 <= seed < 2**64 for seed in seeds) or len(set(seeds)) != len(seeds):
        raise ValueError('independent repetitions require distinct nonnegative u64 seeds')
    groups = set()
    for group in spec['groups']:
        if group['id'] in groups or not re.fullmatch('[a-zA-Z0-9._-]+', group['id']): raise ValueError('invalid or duplicate group id')
        groups.add(group['id'])
        if not group['datasets'] or len(set(group['datasets'])) != len(group['datasets']) or not set(group['datasets']) <= datasets: raise ValueError('group dataset selection differs')
        if group['storage_mode'] != 'native' or group['cpus'] != [16] or group['lane'] != 'journeys': raise ValueError('main cells require native layout, fixed16 CPUs and real journeys')
        if group['selection'] not in ('uniform', 'skew80-20') or group['scenario'] not in json.loads((root / 'bench/ac/million/workload.json').read_text())['offered_multiplier_scenarios']:
            raise ValueError('unknown target selection or population workload scenario')
        if type(group['repeat']) is not int or not 0 < group['repeat'] <= len(spec['seeds']): raise ValueError('invalid repetition count')
        if not group['memory_gib'] or len(set(group['memory_gib'])) != len(group['memory_gib']): raise ValueError('empty or duplicate RAM tier')
        if any(type(value) is not int or value <= 0 for value in group['memory_gib']): raise ValueError('RAM tiers must be positive integer GiB')
        rates = group.get('rates', ['derived'])
        if not rates or len(set(rates)) != len(rates): raise ValueError('empty or duplicate offered rate')
        for rate in rates:
            if rate != 'derived': positive(rate, 'offered rate')
        for key in ('max_pod_bytes', 'cache_source_bytes'): positive(group[key], key)
    # Preserve the established complete-response and queue criteria exactly.
    for key, value in old['measurement'].items():
        if key not in ('warmup_seconds', 'measurement_seconds', 'minimum_offered', 'maximum_inflight', 'queue_capacity_per_worker') and spec['measurement'].get(key) != value:
            raise ValueError(f'campaign changes the established response/workload criterion: {key}')
    for key in ('warmup_seconds', 'measurement_seconds', 'minimum_offered', 'maximum_inflight', 'queue_capacity_per_worker'):
        positive(spec['measurement'][key], key)
    for key in ('minimum_offered', 'maximum_inflight', 'queue_capacity_per_worker'):
        if type(spec['measurement'][key]) is not int: raise ValueError(f'{key} must be an integer count')
    execution = spec['execution']; bounds = spec['stop_rules']
    if spec['host']['instance_type'] != 'r7gd.12xlarge' or spec['host']['physical_memory_gib'] != 384 or spec['host']['physical_vcpus'] != 48:
        raise ValueError('main runner requires the reviewed dedicated physical host')
    if not 128 < spec['host']['physical_memory_gib'] * spec['host']['overall_memory_fraction']:
        raise ValueError('outer study bound must leave room for the complete128GiB preparation unit')
    if execution['preparation_memory_gib'] != 128: raise ValueError('this preparation executor uses the reviewed128GiB units')
    for key in ('phase_timeout_seconds', 'inspection_timeout_seconds', 'maximum_cells'):
        positive(execution[key], key)
    for key in ('startup_timeout_seconds', 'drain_timeout_seconds', 'cell_timeout_seconds'):
        positive(spec['preloaded'][key], key)
    if spec['preloaded']['startup_timeout_seconds'] + 60 >= spec['preloaded']['cell_timeout_seconds']:
        raise ValueError('startup deadline must leave bounded evidence/cleanup time before the cell runtime limit')
    for key in ('runtime_ceiling_seconds', 'disk_floor_bytes', 'result_disk_reserve_bytes', 'result_maximum_bytes', 'result_maximum_file_bytes'):
        positive(bounds[key], key)
    if bounds['runtime_ceiling_seconds'] > 43200 or bounds['disk_floor_bytes'] < 20 * PREP.GIB or bounds['result_disk_reserve_bytes'] < 2 * PREP.GIB:
        raise ValueError('runtime or storage guard exceeds the dedicated-host safety envelope')
    if bounds['result_maximum_bytes'] > 2 * PREP.GIB or bounds['result_maximum_file_bytes'] > 512 * 1024**2:
        raise ValueError('result bounds exceed native-host retrieval contract')
    if spec['preloaded']['cell_timeout_seconds'] >= bounds['runtime_ceiling_seconds'] or execution['phase_timeout_seconds'] >= bounds['runtime_ceiling_seconds']:
        raise ValueError('phase or cell must fit within the whole campaign deadline')
    count = sum(len(group['datasets']) * len(group['memory_gib']) * len(group.get('rates', ['derived'])) * group['repeat'] * 2 for group in spec['groups'])
    if type(execution['maximum_cells']) is not int or not count <= execution['maximum_cells'] <= 10000:
        raise ValueError('frozen matrix exceeds its explicit cell ceiling or bounded planner envelope')
    planned = plan_cells(spec)
    for row in planned:
        CELL.selected_cell(SimpleNamespace(**row, results=Path('/nonexistent-preloaded-admission-result')), spec)
    if set(row['dataset'] for row in planned) != datasets: raise ValueError('population has no declared cell')
    if not spec['shipping_commits'] or any(not re.fullmatch('[0-9a-f]{40}', value) for value in spec['shipping_commits']):
        raise ValueError('explicit merged shipping commit identities required')
    margin = spec.get('statistical_plan', {}).get('practical_equivalence_margin_ratio')
    if margin is not None and (positive(margin, 'practical equivalence margin') <= 1):
        raise ValueError('a frozen practical-equivalence ratio margin must exceed one')
    return planned


def validate_population(corpus, dataset, model):
    """Reconcile every persisted Pod and packed index entry, without a population array."""
    manifest = json.loads((corpus / 'manifest.json').read_text())
    if manifest.get('format') != 'sparq-pod-pack-zstd-v1' or manifest.get('populated') is not True or manifest.get('binary_payloads_included') is not False:
        raise ValueError('not a completely populated metadata corpus')
    if manifest['pods'] != dataset['pods'] or manifest['model'] != model or manifest['config'] != dataset['expected_config']:
        raise ValueError('persisted population, policy model or full generator configuration differs')
    if (corpus / 'updates').exists(): raise ValueError('preparation corpus contains old journals; use a fresh corpus directory')
    sums = dict(records=0, quads=0, source_bytes=0, packed_bytes=0)
    maximum_source = maximum_compressed = count = 0
    with (corpus / 'pod-summaries.jsonl').open() as summaries, (corpus / 'pods.index').open('rb') as index:
        for line in summaries:
            row = json.loads(line)
            raw = index.read(24)
            if len(raw) != 24: raise ValueError('packed index incomplete')
            offset, compressed, source = struct.unpack('<QQQ', raw)
            if row['pod_id'] != count or offset != sums['packed_bytes'] or compressed != row['compressed_bytes'] or source != row['bytes']:
                raise ValueError('persisted Pod sequence/index disagrees with full inventory')
            for key in ('records', 'quads', 'bytes', 'compressed_bytes'):
                if type(row[key]) is not int or row[key] <= 0: raise ValueError('empty or invalid populated Pod')
            sums['records'] += row['records']; sums['quads'] += row['quads']
            sums['source_bytes'] += source; sums['packed_bytes'] += compressed
            maximum_source = max(maximum_source, source); maximum_compressed = max(maximum_compressed, compressed)
            count += 1
        if index.read(1): raise ValueError('packed index has an undeclared trailing Pod')
    expected = sums | {'pods': count, 'index_bytes': count * 24, 'maximum_pod_source_bytes': maximum_source, 'maximum_pod_compressed_bytes': maximum_compressed}
    if count != dataset['pods'] or any(manifest[key] != value for key, value in expected.items()): raise ValueError('complete inventory totals differ from declared population')
    for name, key in [('pods.nqpack', 'packed'), ('pods.index', 'index')]:
        path = corpus / name
        if path.stat().st_size != manifest[key + '_bytes'] or PREP.digest(path) != manifest[key + '_sha256']: raise ValueError('persisted source payload size/hash differs')
    return manifest, BASE.representatives(corpus / 'pod-summaries.jsonl')


class MainPreparation(PREP.Preparation):
    def __init__(self, owner):
        args = SimpleNamespace(**vars(owner.args), server_cpus=','.join(map(str, owner.spec['host']['server_cpu_pool'])))
        super().__init__(args, {'proposed_bounds': {'pilot_total_wall_seconds': owner.spec['stop_rules']['runtime_ceiling_seconds'], 'per_command_timeout_seconds': owner.spec['execution']['phase_timeout_seconds']}})
        self.owner = owner; self.deadline = owner.deadline

    def guard(self): self.owner.guard()


class MainCell(CELL.PreloadedCell):
    def __init__(self, owner, selection, directory):
        self.owner = owner
        super().__init__(SimpleNamespace(**vars(owner.args) | selection | {'results': directory}))

    def guard(self, estimate=0):
        self.owner.guard(estimate)
        super().guard(estimate)

    def run(self, command, log, timeout=None):
        # Preserve the shared planner/load/drain argv and all-offered evidence.
        # Offline audits also use the native archive; their work is never timed HTTP.
        if len(command) > 1 and command[1] == 'audit':
            command = command + ['--storage-mode', 'native']
            label = log.stem + '-native-offline'
            status = 0
            try: self.owner.preparation.phase(command, label)
            except RuntimeError:
                self.owner.guard()
                status = 1
            source = self.owner.args.results / (label + '.log')
            if source.exists(): shutil.copyfile(source, log)
            return status
        timeout = min(timeout or self.owner.spec['execution']['phase_timeout_seconds'], self.owner.remaining())
        return super().run(command, log, timeout)

    def start_server(self, *args):
        self.owner.preparation.require_exclusive_jobs()
        started = time.monotonic()
        complete = False; unit = None
        try:
            unit = super().start_server(*args); complete = True
            return unit
        except CELL.ADMISSION.AdmissionFailure as error:
            unit = error.record['unit']; raise
        finally:
            BASE.write_json(self.results / (args[-1] + '-startup-boundary.json'), {
                'complete': complete, 'elapsed_seconds': time.monotonic() - started,
                'cold_os_caches_before_launch': True,
                'boundary': 'After the inherited checked sync/drop_caches command and exclusive-job check, before systemd launch through complete readiness/resource capture or failed admission capture and owned-unit cleanup; no HTTP workload yet.',
                'unit': unit or self.current_server,
            })

    def admission_snapshot(self, unit):
        try: return self.owner.preparation.snapshot(unit)
        except ValueError as error:
            # An exited unit's cgroup may already have been destroyed. Preserve
            # the missing snapshot explicitly; its ExecStopPost record is the
            # authoritative local OOM evidence, not an invented zero counter.
            return {'systemd': self.owner.preparation.properties(unit), 'cgroup': {},
                    'process': {}, 'errors': [str(error)]}

    def resource(self, unit, label):
        super().resource(unit, label)
        if label.endswith('-before'):
            drain = json.loads((self.results / (label + '-drain.jsonl')).read_text())
            if any(row['state']['activations']['journal_entries_replayed'] for row in drain['workers']):
                raise ValueError('independent run did not begin from the pristine immutable population')
        snapshot = self.owner.preparation.snapshot(unit)
        BASE.write_json(self.results / (label + '-native-accounting.json'), snapshot)
        try: PREP.validate_resources(snapshot, self.owner.active_selection['memory_gib'] * PREP.GIB)
        except (ValueError, KeyError) as error:
            path = self.results / (label + '-resources.json')
            values = json.loads(path.read_text()); values['error'] = str(error); BASE.write_json(path, values)

class Campaign:
    def __init__(self, args, spec, planned):
        self.args = args; self.spec = spec; self.started = getattr(args, 'started_monotonic', time.monotonic())
        self.deadline = self.started + spec['stop_rules']['runtime_ceiling_seconds']
        self.populations = []; self.cells = [dict(selection=row, status='unattempted') for row in planned]
        self.preparation = MainPreparation(self); self.active_cell = None; self.active_selection = None
        self.source_quarantined = False

    def remaining(self): return max(.001, self.deadline - time.monotonic() - 30)

    def guard(self, estimate=0):
        bounds = self.spec['stop_rules']
        if time.monotonic() + estimate >= self.deadline - 30: raise TimeoutError('frozen campaign wall deadline')
        if shutil.disk_usage(self.args.corpora).free < bounds['disk_floor_bytes']: raise RuntimeError('frozen data0 disk floor')
        if shutil.disk_usage(self.args.results).free < bounds['result_disk_reserve_bytes']: raise RuntimeError('frozen result disk reserve')
        sizes = [path.stat().st_size for path in self.args.results.rglob('*') if path.is_file()]
        if sum(sizes) > bounds['result_maximum_bytes'] or any(size > bounds['result_maximum_file_bytes'] for size in sizes): raise RuntimeError('frozen result retrieval limit')

    @contextmanager
    def inspect(self):
        self.guard()
        with PREP.bounded_inspection(min(self.spec['execution']['inspection_timeout_seconds'], self.remaining())): yield

    def verify_inputs(self):
        with self.inspect():
            if PREP.digest(self.args.campaign) != self.args.campaign_sha256 or PREP.digest(self.args.binary) != self.args.binary_sha256:
                raise ValueError('frozen campaign or binary changed after preparation')
            if subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip() != self.args.source_commit or subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT):
                raise ValueError('source checkout changed after campaign admission')

    def prepare(self, dataset, model):
        label = dataset['id'] + '-' + model; corpus = self.args.corpora / label
        if corpus.is_symlink(): raise ValueError('corpus directories cannot be symlink aliases')
        record = {'dataset': dataset['id'], 'model': model, 'status': 'partial'}
        self.populations.append(record); corpus.mkdir(exist_ok=True)
        with (corpus / 'preloaded-cell.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            if not (corpus / 'manifest.json').exists():
                command = [str(self.args.binary), 'pack', '--corpus', str(corpus), '--pods', str(dataset['pods']), '--model', model]
                command += ['--config-file', str(ROOT / dataset['config_file'])] if dataset.get('config_file') else ['--profile', dataset['profile']]
                record['generation'] = self.preparation.phase(command, label + '-generate')
            else: record['generation'] = {'status': 'reused', 'new_preparation_time': None}
            with self.inspect(): manifest, representatives = validate_population(corpus, dataset, model)
            record['manifest'] = manifest; record['representatives'] = representatives
            source_copy = self.args.results / (label + '-source-manifest.json')
            shutil.copyfile(corpus / 'manifest.json', source_copy)
            with self.inspect(): record['source_manifest_sha256'] = PREP.digest(source_copy)
            maximum = max(group['max_pod_bytes'] for group in self.spec['groups'] if dataset['id'] in group['datasets'])
            if any(group['max_pod_bytes'] < manifest['maximum_pod_source_bytes'] for group in self.spec['groups'] if dataset['id'] in group['datasets']):
                raise ValueError('a frozen group excludes an actual heavy Pod via its source-size cap')
            sidecar = corpus / 'pods.native-manifest.json'
            if not sidecar.exists():
                record['cold_origin'] = self.preparation.cold_origin()
                record['native_preparation'] = self.preparation.phase([str(self.args.binary), 'prepare-native', '--corpus', str(corpus), '--native-compressed', 'true', '--max-pod-bytes', str(maximum)], label + '-prepare')
            else: record['native_preparation'] = {'status': 'reused', 'new_preparation_time': None}
            with self.inspect():
                native = json.loads(sidecar.read_text()); archive = corpus / 'pods.native'
                if native['pods'] != dataset['pods'] or native['compressed'] is not True or native['source_manifest_sha256'] != PREP.digest(corpus / 'manifest.json') or native['archive_bytes'] != archive.stat().st_size or native['archive_sha256'] != PREP.digest(archive):
                    raise ValueError('complete native archive binding differs')
                record['native_manifest'] = native; record['storage'] = PREP.storage_inventory(corpus)
            self.preparation.cold_origin()
            try:
                record['oracle'] = self.preparation.phase([str(self.args.binary), 'verify', '--storage-mode', 'native', '--corpus', str(corpus), '--max-pod-bytes', str(maximum), '--verify-pod-ids', ','.join(str(row['pod']) for row in representatives), '--workload-file', str(ROOT / 'bench/ac/million/workload.json')], label + '-oracle')
            except RuntimeError:
                self.source_quarantined = 'mismatch' in BASE.tail(self.args.results / (label + '-oracle.log'))
                raise
            BASE.write_json(self.args.results / (label + '-population.json'), record | {'status': 'complete'})
            # Retain the full per-Pod inventory, not only aggregate extrema.
            self.preparation.phase(['zstd', '-q', str(corpus / 'pod-summaries.jsonl'), '-o', str(self.args.results / (label + '-pod-summaries.jsonl.zst'))], label + '-inventory')
            record['status'] = 'complete'

    def execute(self):
        status = 'partial'; error = None; cleanup_errors = []
        try:
            self.preparation.require_exclusive_jobs(); self.guard()
            if PREP.auth_requires_provision(self.args.auth): self.preparation.phase([str(self.args.binary), 'auth', '--auth-dir', str(self.args.auth)], 'auth-provision')
            secret = self.args.auth / 'preloaded-control-token'
            if not secret.exists():
                with os.fdopen(os.open(secret, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w') as stream: stream.write(secrets.token_hex(32))
            for dataset in self.spec['corpora']:
                for model in dataset['models']: self.prepare(dataset, model)
            for index, row in enumerate(self.cells):
                self.guard(); row['status'] = 'partial'; self.active_selection = row['selection']
                self.verify_inputs()
                directory = self.args.results / f'cell-{index:05d}'
                self.active_cell = MainCell(self, row['selection'], directory)
                cell = self.active_cell; dataset, group, rate, label = cell.selection
                row['directory'] = directory.name; row['label'] = label
                corpus = self.args.corpora / (dataset['id'] + '-' + row['selection']['model'])
                with (corpus / 'preloaded-cell.lock').open('a') as lock:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    self.preparation.require_exclusive_jobs()
                    try:
                        with PREP.bounded_inspection(min(self.spec['preloaded']['cell_timeout_seconds'], self.remaining())):
                            row['summary'] = cell.cell(dataset, corpus, row['selection']['model'], group, row['selection']['memory_gib'], 16, rate, row['selection']['replicate'])
                    except CELL.ADMISSION.AdmissionFailure as failure:
                        CELL.ADMISSION.classify(failure.record)
                        row['admission'] = failure.record
                    cell.stop_server(); self.active_cell = None
                if 'admission' in row:
                    row['status'] = 'admission-failed'
                else:
                    if row['summary'].get('dataset_quarantined') or not row['summary']['correctness_passed']: raise RuntimeError('unverified mutation state; source retained and campaign stopped')
                    row['status'] = 'complete'
                self.active_selection = None
                print(json.dumps({'record_type': 'preloaded-campaign-progress', 'completed_cells': index + 1, 'total_cells': len(self.cells)}), flush=True)
            status = 'complete'
        except BaseException as failure:
            error = str(failure)
            if self.active_selection is not None:
                next(row for row in self.cells if row['selection'] == self.active_selection)['error'] = error
            elif self.populations and self.populations[-1]['status'] != 'complete':
                self.populations[-1]['error'] = error
        finally:
            for stop in [self.preparation.stop] + ([self.active_cell.stop_server] if self.active_cell else []):
                try: stop()
                except (OSError, subprocess.SubprocessError) as failure: cleanup_errors.append(str(failure))
            if cleanup_errors: status = 'partial'
            try:
                with PREP.bounded_inspection(5):
                    for path in self.args.results.glob('cell-*/campaign-events.jsonl'):
                        with path.open() as stream:
                            if any(json.loads(line).get('record_type') == 'correctness-quarantine' for line in stream): self.source_quarantined = True
            except (OSError, ValueError, TimeoutError) as failure:
                cleanup_errors.append('quarantine event capture incomplete: ' + str(failure)); status = 'partial'
            result = {'record_type': 'native-preloaded-campaign-result', 'status': status,
                'source_commit': self.args.source_commit, 'binary_build_source_commit': self.args.binary_source_commit,
                'binary_sha256': self.args.binary_sha256, 'campaign_sha256': self.args.campaign_sha256,
                'error': error, 'cleanup_errors': cleanup_errors, 'source_quarantined': self.source_quarantined, 'populations': self.populations,
                'unattempted_populations': [{'dataset': d['id'], 'model': m} for d in self.spec['corpora'] for m in d['models'] if not any(r['dataset'] == d['id'] and r['model'] == m for r in self.populations)],
                'cells': self.cells, 'elapsed_seconds': time.monotonic() - self.started,
                'scope': 'Execution completion is not capacity admission; preserve all offered requests, failed guards, readiness/resource evidence and independent audits for final analysis.'}
            BASE.write_json(self.args.results / 'campaign-result.json', result)
            try:
                if cleanup_errors: raise RuntimeError('cleanup/accounting incomplete; do not bind possibly active outputs')
                with PREP.bounded_inspection(20):
                    # Incomplete cells retain closed raw logs too. Compress them
                    # losslessly, never trim schedules to meet retrieval limits.
                    for pattern in ('*.jsonl', '*.log'):
                        for path in sorted(self.args.results.rglob(pattern)): BASE.compress(path)
                    entries = []
                    for path in sorted(self.args.results.rglob('*')):
                        if path.is_symlink(): raise ValueError('result manifest cannot include symlinks')
                        if path.is_file(): entries.append(f'{PREP.digest(path)}  {path.relative_to(self.args.results)}\n')
                    (self.args.results / 'MANIFEST.sha256').write_text(''.join(entries))
            except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as failure:
                status = 'partial'; result.update(status=status, finalization_error=str(failure))
                BASE.write_json(self.args.results / 'campaign-result.json', result)
        if status != 'complete': raise RuntimeError('preloaded campaign partial; inspect retained result and raw evidence')


def main():
    parser = argparse.ArgumentParser()
    for name in ('campaign', 'binary', 'corpora', 'auth', 'results'): parser.add_argument('--' + name, type=Path, required=True)
    for name in ('campaign-sha256', 'source-commit', 'binary-source-commit', 'binary-sha256'): parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    args.started_monotonic = time.monotonic()
    # This is the dedicated host's maximum execution envelope, including preflight.
    with PREP.bounded_inspection(43200 - 30):
        for name, length in [('source_commit', 40), ('binary_source_commit', 40), ('campaign_sha256', 64), ('binary_sha256', 64)]:
            if not re.fullmatch('[0-9a-f]{' + str(length) + '}', getattr(args, name)): raise ValueError('malformed explicit identity')
        PREP.within(ROOT, PREP.DATA)
        for name in ('binary', 'corpora', 'auth'): setattr(args, name, PREP.within(getattr(args, name), PREP.DATA))
        args.results = PREP.within(args.results, PREP.RESULTS)
        with PREP.bounded_inspection(30):
            if args.campaign.stat().st_size > 2 * 1024**2 or PREP.digest(args.campaign) != args.campaign_sha256: raise ValueError('campaign size/hash differs')
            spec = json.loads(args.campaign.read_text())
            runtime = positive(spec['stop_rules']['runtime_ceiling_seconds'], 'campaign runtime')
            if runtime > 43200: raise ValueError('campaign exceeds dedicated host runtime envelope')
        with PREP.bounded_inspection(runtime - (time.monotonic() - args.started_monotonic) - 30):
            execute_main(args, spec)


def execute_main(args, spec):
    if PREP.digest(args.binary) != args.binary_sha256: raise ValueError('binary hash differs')
    if args.source_commit != args.binary_source_commit: raise ValueError('main campaign requires the binary built from this exact source revision')
    if subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip() != args.source_commit or subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT): raise ValueError('exact clean source checkout required')
    planned = validate_campaign(spec)
    if spec['source_commit'] != args.source_commit or spec['binary_build_source_commit'] != args.binary_source_commit or spec['binary_sha256'] != args.binary_sha256:
        raise ValueError('frozen campaign source/binary identities differ from explicit execution inputs')
    for revision in spec['shipping_commits']: subprocess.run(['git', 'merge-base', '--is-ancestor', revision, args.source_commit], cwd=ROOT, check=True)
    if not PREP.DATA.is_mount() or not Path('/sys/fs/cgroup/cgroup.controllers').exists(): raise ValueError('dedicated data0 and Linux cgroup v2 required')
    if os.cpu_count() != spec['host']['physical_vcpus']: raise ValueError('observed CPU count differs from declared physical host')
    if args.corpora.is_relative_to(ROOT) or args.auth.is_relative_to(ROOT) or args.corpora.is_relative_to(args.auth) or args.auth.is_relative_to(args.corpora):
        raise ValueError('source, corpus storage and authentication directories must be separate')
    if args.results.exists() and any(args.results.iterdir()): raise ValueError('never overwrite or resume a prior result directory')
    args.results.mkdir(parents=True, exist_ok=True); args.corpora.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(args.campaign, args.results / 'frozen-campaign.json'); args.campaign = args.results / 'frozen-campaign.json'
    with (PREP.DATA / 'preparation-pilot.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        outer = int(spec['host']['physical_memory_gib'] * PREP.GIB * spec['host']['overall_memory_fraction'])
        subprocess.run(['sudo', 'systemctl', 'set-property', '--runtime', 'sparq-pod-bench.slice', f'MemoryMax={outer}', 'MemorySwapMax=0'], check=True, timeout=15)
        campaign = Campaign(args, spec, planned)
        def cancelled(_signal, _frame): raise InterruptedError('campaign cancelled; preserve evidence and stop owned units')
        signal.signal(signal.SIGTERM, cancelled); signal.signal(signal.SIGINT, cancelled)
        campaign.execute()


if __name__ == '__main__': main()
