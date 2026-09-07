#!/usr/bin/env python3
"""[GPT-6] Execute the source-bound preparation proposal without timed load."""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import subprocess
import time

SPEC = importlib.util.spec_from_file_location('preloaded_cell', Path(__file__).with_name('run-cell.py'))
CELL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CELL)
ROOT = CELL.ROOT
GIB = 1024**3
DATA = Path('/mnt/sparq-native/data0')
RESULTS = Path('/var/tmp/sparq-pod-study')
EXECUTOR_ONLY_PATHS = frozenset({
    'bench/ac/preloaded/run-preparation.py', 'bench/ac/preloaded/test_preparation.py',
    'bench/ac/preloaded/README.md', 'bench/ac/preloaded/remote-checks.json',
    'skills/solid-lws-server/SKILL.md',
})


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write_json(path, value):
    CELL.BASE.write_json(path, value)


@contextmanager
def bounded_inspection(seconds):
    """Interrupt blocking filesystem reads as well as cooperative Python work."""
    if seconds <= 0: raise TimeoutError('inspection has no remaining time budget')
    previous_handler = signal.getsignal(signal.SIGALRM)
    previous_timer = signal.getitimer(signal.ITIMER_REAL)
    started = time.monotonic()
    def expired(_signal, _frame): raise TimeoutError('bounded preparation inspection expired')
    signal.signal(signal.SIGALRM, expired)
    limit = min(seconds, previous_timer[0]) if previous_timer[0] > 0 else seconds
    signal.setitimer(signal.ITIMER_REAL, limit)
    try: yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous_handler)
        if previous_timer[0] > 0:
            remaining = previous_timer[0] - (time.monotonic() - started)
            if remaining > 0: signal.setitimer(signal.ITIMER_REAL, remaining, previous_timer[1])


def within(path, parent):
    path = path.resolve(); parent = parent.resolve()
    if path == parent or not path.is_relative_to(parent):
        raise ValueError(f'path must be strictly below {parent}: {path}')
    return path


def validate_proposal(proposal, root=ROOT):
    if proposal['schema_version'] != 1 or not proposal['status'].startswith('prospective footprint proposal only;'):
        raise ValueError('wrong preparation proposal kind')
    for relative, expected in proposal['bindings'].items():
        path = within(root / relative, root)
        if digest(path) != expected:
            raise ValueError(f'proposal source binding differs: {relative}')
    expected_definitions = {r['id']: r for r in json.loads((root / 'bench/ac/million/campaign-20260906.json').read_text())['corpora']}
    expected_rows = {(r['dataset'], r['model']): r for r in json.loads((root / 'research/solid-pod-scale-main.json').read_text())['corpora']}
    seen = set()
    for cell in proposal['cells']:
        identity = (cell['dataset'], cell['model'])
        if identity in seen or cell['definition'] != expected_definitions[cell['dataset']]:
            raise ValueError('duplicate population or changed corpus definition')
        seen.add(identity)
        prior = expected_rows[identity]
        if not prior['storage_inventory_consistent']:
            raise ValueError('prior storage evidence inconsistent')
        if any(prior['manifest'][key] != value for key, value in cell['prior_observations'].items()):
            raise ValueError('proposal changes prior population observations')
        if cell['existing_intensity_extrema'] != prior['verification']['expected_representatives']:
            raise ValueError('proposal changes observed intensity representatives')
        config = cell['definition'].get('config_file')
        if config and digest(within(root / config, root)) != cell['config_sha256']:
            raise ValueError('corpus configuration hash differs')
    if len(seen) != 12 or any((dataset, other) not in seen for dataset, model in seen for other in ('wac', 'acp')):
        raise ValueError('complete paired preparation proposal required')
    environment = proposal['proposed_environment']
    if environment['server_cpus'] != 16 or environment['disjoint_client_cpus'] != 4 or environment['preparation_and_reference_startup_memory_gib'] != 128:
        raise ValueError('preparation CPU/RAM allocation differs')
    bounds = proposal['proposed_bounds']
    if not 0 < bounds['per_command_timeout_seconds'] <= 900 or not 0 < bounds['pilot_total_wall_seconds'] <= 3600:
        raise ValueError('proposal runtime bounds exceed the reviewed pilot')


def source_equivalence(binary_revision, executor_revision, root=ROOT):
    output = subprocess.check_output(['git', 'diff', '--name-only', '--no-renames', '-z',
                                      binary_revision, executor_revision, '--'], cwd=root)
    changed = sorted(path.decode('utf-8') for path in output.split(b'\0') if path)
    if set(changed) - EXECUTOR_ONLY_PATHS:
        raise ValueError('binary and executor sources differ outside the explicit Python/docs-only allowlist')
    return {'passed': True, 'method': 'git diff --name-only --no-renames -z; explicit reviewed executor/docs-only allowlist',
            'binary_build_source_commit': binary_revision, 'executor_source_commit': executor_revision,
            'allowed_changed_paths': sorted(EXECUTOR_ONLY_PATHS), 'actual_changed_paths': changed}


def auth_requires_provision(directory):
    """Accept an empty prepared directory, but never overwrite partial key state."""
    keys = [directory / name for name in ('client-secrets.json', 'issuer-public.json')]
    present = [path.exists() for path in keys]
    if any(present) and not all(present):
        raise ValueError('authentication directory contains incomplete key state')
    if all(present) and not all(path.is_file() and not path.is_symlink() for path in keys):
        raise ValueError('authentication keys must be regular files')
    return not any(present)


def validate_corpus(corpus, cell):
    manifest = json.loads((corpus / 'manifest.json').read_text())
    if manifest.get('populated') is not True or manifest.get('model') != cell['model'] or manifest.get('format') != 'sparq-pod-pack-zstd-v1':
        raise ValueError('persisted corpus is not the declared populated policy model')
    if any(manifest.get(key) != value for key, value in cell['prior_observations'].items()):
        raise ValueError('regenerated corpus differs from prior complete population')
    for filename, size, checksum in [('pods.nqpack', 'packed_bytes', 'packed_sha256'), ('pods.index', 'index_bytes', 'index_sha256')]:
        path = corpus / filename
        if path.stat().st_size != manifest[size] or digest(path) != manifest[checksum]:
            raise ValueError(f'complete source payload differs: {filename}')
    actual = CELL.BASE.representatives(corpus / 'pod-summaries.jsonl')
    if actual != cell['existing_intensity_extrema']:
        raise ValueError('regenerated intensity extrema differ')
    if (corpus / 'updates').exists() and any((corpus / 'updates').rglob('*.jsonl')):
        raise ValueError('preparation baseline contains mutation journals; no reset is authorized here')
    return manifest


def storage_inventory(corpus):
    files = []
    for path in sorted(corpus.rglob('*')):
        if path.is_symlink():
            raise ValueError('corpus inventory cannot follow symlinks')
        stat = path.stat()
        files.append({'path': str(path.relative_to(corpus)), 'kind': 'directory' if path.is_dir() else 'file',
                      'logical_bytes': stat.st_size, 'allocated_bytes': stat.st_blocks * 512})
    return {'entries': files, 'files': sum(row['kind'] == 'file' for row in files),
            'directories_including_root': 1 + sum(row['kind'] == 'directory' for row in files),
            'file_logical_bytes': sum(row['logical_bytes'] for row in files if row['kind'] == 'file'),
            'allocated_bytes_including_directories': corpus.stat().st_blocks * 512 + sum(row['allocated_bytes'] for row in files)}


def validate_resources(snapshot, limit=128 * GIB):
    if snapshot.get('errors'):
        raise ValueError('required cgroup/process resource evidence is incomplete')
    values = snapshot['cgroup']
    events = dict(line.split() for line in values['memory.events'].splitlines())
    memory = dict(line.split() for line in values['memory.stat'].splitlines())
    cpu = dict(line.split() for line in values['cpu.stat'].splitlines())
    if int(values['memory.max']) != limit or int(values['memory.swap.max']) != 0 or int(values['memory.peak']) > limit:
        raise ValueError('actual cgroup memory boundary differs or was exceeded')
    if int(events['oom']) or int(events['oom_kill']) or not {'anon', 'file'} <= memory.keys() or 'usage_usec' not in cpu:
        raise ValueError('OOM event or incomplete memory/CPU accounting')


def history_projections(proposal, records, prior_rows):
    """Label simple observed-prefix scaling as models, never measured capacity."""
    observed = {(row['dataset'], row['model']): row for row in records if row['status'] == 'complete'}
    definitions = {(row['dataset'], row['model']): row for row in proposal['cells']}
    models = []
    for model in ('wac', 'acp'):
        result = {'model': model, 'available': False}
        samples = []
        for dataset in ('history-8', 'history-64'):
            row = observed.get((dataset, model))
            if row is None: break
            prior = definitions[dataset, model]['prior_observations']
            native = next(entry['logical_bytes'] for entry in row['storage']['entries'] if entry['path'] == 'pods.native')
            memory = dict(line.split() for line in row['after_drain_resources']['cgroup']['memory.stat'].splitlines())
            samples.append({'dataset': dataset, 'pods': prior['pods'], 'records': prior['records'],
                'native_bytes': native, 'retained_cgroup_anon_proxy_bytes': int(memory['anon']),
                'startup_seconds': row['startup']['elapsed_seconds_including_unit_start_and_observation'],
                'prepare_seconds': row['preparation'].get('elapsed_seconds_including_unit_start_and_observation')})
        if len(samples) == 2:
            result.update(available=True, observed_samples=samples, targets=[])
            for pods in (1000, 10000, 100000, 1000000):
                target = {'pods': pods, 'status': 'modeled-unmeasured-native-population', 'ranges': {}}
                for key in ('native_bytes', 'retained_cgroup_anon_proxy_bytes', 'startup_seconds', 'prepare_seconds'):
                    values = [sample[key] / sample['pods'] * pods for sample in samples if sample[key] is not None]
                    target['ranges'][key] = [math.ceil(min(values)), math.ceil(max(values))] if len(values) == 2 else None
                prior = next((row for row in prior_rows if row['dataset'] == f'history-{pods}' and row['model'] == model), None)
                target['previous_packed_source_volume'] = {key: prior['manifest'][key] for key in ('pods', 'records', 'quads', 'source_bytes')} if prior else None
                result['targets'].append(target)
        else:
            result['reason'] = 'Both complete history8 and heavy-tail history64 readiness footprints are required; no projection from normal prefixes alone.'
        models.append(result)
    return {'record_type': 'prospective-native-history-projections', 'proposal_sha256': None, 'models': models,
        'method': 'For each metric, multiply the smaller/larger observed per-Pod value across history8/history64 by each prospective population. No fitting of a safe upper bound or statistical confidence interval.',
        'uncertainty': 'Two deterministic prefixes do not validate the population distribution. History8 lacks4x/20x classes; history64 contains them with finite-sample weights. Larger populations, heavy users and entropy may fall outside this scenario envelope. Startup/preparation overhead and scaling can be nonlinear.',
        'memory_scope': 'Cgroup anon after complete zero-workload drain is a retained-heap proxy including stacks/allocator/other anonymous state, not exact heap allocation. Dictionary term/record bytes are mapped; session caches and future write overlays can grow after readiness.',
        'decision_scope': 'Use only to budget the next real retained-history preparation/admission within native storage, runtime and cost limits. No population is admitted by projection and retained histories are not capped at1000. Every larger claimed population must actually be generated/prepared in full.'}


class Preparation:
    def __init__(self, args, proposal):
        self.args = args; self.proposal = proposal
        self.results = args.results; self.current = None; self.sequence = 0
        self.started = time.monotonic()
        self.deadline = self.started + proposal['proposed_bounds']['pilot_total_wall_seconds']
        self.timeout = proposal['proposed_bounds']['per_command_timeout_seconds']
        self.records = []

    def guard(self):
        if time.monotonic() >= self.deadline - 30:
            raise TimeoutError('preparation-only pilot wall limit reached')
        if shutil.disk_usage(self.args.corpora).free < self.args.disk_floor_gib * GIB:
            raise RuntimeError('data0 free disk floor reached')
        if shutil.disk_usage(self.results).free < 2 * GIB:
            raise RuntimeError('results filesystem reserve reached')
        sizes = [p.stat().st_size for p in self.results.rglob('*') if p.is_file()]
        if sum(sizes) > 2 * GIB or any(size > 512 * 1024**2 for size in sizes):
            raise RuntimeError('result retrieval bound reached')

    def properties(self, unit):
        output = subprocess.check_output(['sudo', 'systemctl', 'show', unit, '--property=ActiveState,SubState,Result,ExecMainStatus,ExecMainCode,MainPID,ControlGroup,MemoryPeak,CPUUsageNSec,ExecMainStartTimestampMonotonic,ExecMainExitTimestampMonotonic'], text=True, timeout=10)
        return dict(line.split('=', 1) for line in output.splitlines() if '=' in line)

    def inspect(self, operation, *args):
        self.guard()
        with bounded_inspection(min(self.timeout, self.deadline - time.monotonic() - 30)):
            return operation(*args)

    def require_exclusive_jobs(self):
        own = Path('/proc/self/cgroup').read_text().splitlines()
        own_units = {line.rsplit('/', 1)[-1] for line in own if line.endswith('.service')}
        output = subprocess.check_output(['sudo', 'systemctl', 'list-units', '--type=service',
            '--state=active,activating,deactivating', '--no-legend', '--plain', '--no-pager',
            'sparq-native-job-*', 'sparq-pod-preloaded-*', 'sparq-pod-server-*'], text=True, timeout=10)
        others = {line.split()[0] for line in output.splitlines() if line.split()} - own_units
        if self.current: others.discard(self.current)
        if others: raise RuntimeError(f'another native workload is active: {sorted(others)}')

    def cold_origin(self):
        self.guard(); self.require_exclusive_jobs()
        subprocess.run(['sudo', 'sh', '-c', 'sync; echo 3 > /proc/sys/vm/drop_caches'], check=True, timeout=60)
        return {'clean_os_caches_dropped_before_unit': True,
                'scope': 'dedicated exclusive host; source/archive page faults occur inside the subsequent limited unit'}

    def snapshot(self, unit):
        properties = self.properties(unit)
        result = {'systemd': properties, 'cgroup': {}, 'process': {}, 'errors': [],
                  'host': {}, 'host_errors': [], 'study_slice': {}}
        path = properties.get('ControlGroup', '')
        if 'sparq-pod-bench.slice' not in path.split('/') or not path.endswith('/' + unit):
            raise ValueError('cgroup outside the owned pilot unit')
        for name in ('memory.current', 'memory.peak', 'memory.max', 'memory.swap.max', 'memory.events', 'memory.stat', 'cpu.stat', 'io.stat', 'pids.current'):
            try: result['cgroup'][name] = (Path('/sys/fs/cgroup') / path.lstrip('/') / name).read_text()
            except OSError as error: result['errors'].append(f'{name}: {error}')
        for name in ('meminfo', 'pressure/memory', 'loadavg'):
            try: result['host'][name] = (Path('/proc') / name).read_text()
            except OSError as error: result['host_errors'].append(f'{name}: {error}')
        parent = (Path('/sys/fs/cgroup') / path.lstrip('/')).parent
        for name in ('memory.current', 'memory.peak', 'memory.max', 'memory.events', 'memory.stat', 'cpu.stat'):
            try: result['study_slice'][name] = (parent / name).read_text()
            except OSError as error: result['host_errors'].append(f'study_slice/{name}: {error}')
        result['host_scope'] = 'Host MemAvailable includes OS and all processes; study-slice peak is cumulative across pilot phases. Physical host size is distinct from this unit\'s128GiB quota; no smaller physical-machine fit is inferred.'
        pid = int(properties.get('MainPID', '0'))
        if pid:
            for name in ('status', 'io', 'smaps_rollup', 'stat'):
                try: result['process'][name] = Path(f'/proc/{pid}/{name}').read_text()
                except OSError as error: result['errors'].append(f'proc/{name}: {error}')
            try: result['process']['open_descriptors'] = len(list(Path(f'/proc/{pid}/fd').iterdir()))
            except OSError as error: result['errors'].append(f'proc/fd: {error}')
        return result

    def stop(self):
        if self.current:
            unit = self.current
            subprocess.run(['sudo', 'systemctl', 'stop', unit], check=True, timeout=30)
            subprocess.run(['sudo', 'systemctl', 'reset-failed', unit], check=False, capture_output=True, timeout=10)
            self.current = None

    def launch(self, argv, label):
        self.guard(); self.sequence += 1
        unit = f'sparq-native-job-pilot-{os.getpid()}-{self.sequence}.service'
        self.current = unit
        log = self.results / (label + '.log')
        if log.exists(): raise ValueError('phase output already exists')
        seconds = min(self.timeout, int(self.deadline - time.monotonic() - 30))
        command = ['sudo', 'systemd-run', '--quiet', f'--unit={unit}', '--slice=sparq-pod-bench.slice',
            f'--uid={os.getuid()}', f'--gid={os.getgid()}', f'--working-directory={ROOT}',
            '--property=Type=exec', '--property=RemainAfterExit=yes', '--property=KillMode=control-group',
            '--property=TimeoutStopSec=15', f'--property=RuntimeMaxSec={seconds}', '--property=MemoryAccounting=yes',
            '--property=CPUAccounting=yes', '--property=IOAccounting=yes', f'--property=MemoryMax={128 * GIB}',
            '--property=MemorySwapMax=0', f'--property=StandardOutput=append:{log}', f'--property=StandardError=append:{log}',
            'taskset', '-c', self.args.server_cpus, *argv]
        subprocess.run(command, check=True, timeout=15)
        return unit, log, time.monotonic() + seconds

    def phase(self, argv, label, ready=False):
        started = time.monotonic(); unit, log, deadline = self.launch(argv, label)
        report = {'phase': label, 'unit': unit, 'argv': argv, 'status': 'incomplete'}
        heartbeat = 0
        try:
            while True:
                self.guard()
                if time.monotonic() >= deadline: raise TimeoutError(f'{label} command timeout')
                properties = self.properties(unit)
                if ready and log.exists():
                    rows = []
                    for line in log.read_text().splitlines():
                        try: rows.append(json.loads(line))
                        except json.JSONDecodeError: pass
                    if any(row.get('record_type') == 'server-ready' for row in rows):
                        report['readiness'] = [row for row in rows if row.get('record_type') == 'all-population-ready']
                        report['archive_verification'] = [row for row in rows if row.get('record_type') == 'native-archive-verified']
                        break
                if properties.get('SubState') == 'exited' or properties.get('ActiveState') in ('failed', 'inactive'):
                    if ready or properties.get('Result') != 'success' or properties.get('ExecMainStatus') != '0':
                        raise RuntimeError(f'{label} did not complete: {properties}')
                    break
                if time.monotonic() >= heartbeat:
                    print(json.dumps({'record_type': 'preparation-heartbeat', 'phase': label, 'elapsed_seconds': time.monotonic() - started}), flush=True)
                    heartbeat = time.monotonic() + 30
                time.sleep(.25)
            report['resources'] = self.snapshot(unit)
            validate_resources(report['resources'])
            report['elapsed_seconds_including_unit_start_and_observation'] = time.monotonic() - started
            report['status'] = 'complete'
            return report
        except BaseException as error:
            report['error'] = str(error)
            try: report['resources'] = self.snapshot(unit)
            except (OSError, ValueError, subprocess.SubprocessError) as capture: report['resource_capture_error'] = str(capture)
            raise
        finally:
            write_json(self.results / (label + '.json'), report)
            if not ready or report['status'] != 'complete': self.stop()

    def cell(self, cell):
        label = cell['dataset'] + '-' + cell['model']; corpus = self.args.corpora / label
        record = {'dataset': cell['dataset'], 'model': cell['model'], 'stage': cell['stage'], 'status': 'incomplete'}
        self.records.append(record)
        corpus.mkdir(exist_ok=True)
        with (corpus / 'preloaded-cell.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            if not (corpus / 'manifest.json').exists():
                definition = cell['definition']
                argv = [str(self.args.binary), 'pack', '--corpus', str(corpus), '--pods', str(definition['pods']), '--model', cell['model']]
                argv += ['--config-file', str(ROOT / definition['config_file'])] if definition.get('config_file') else ['--profile', definition['profile']]
                record['generation'] = self.phase(argv, label + '-generate')
            else: record['generation'] = {'status': 'reused-persisted-corpus', 'elapsed_seconds': None}
            manifest = self.inspect(validate_corpus, corpus, cell)
            write_json(self.results / (label + '-source-manifest.json'), manifest)
            native = corpus / 'pods.native'; sidecar = corpus / 'pods.native-manifest.json'
            maximum = str(self.proposal['proposed_environment']['max_pod_bytes'])
            if not sidecar.exists():
                record['preparation_cold_origin'] = self.cold_origin()
                record['preparation'] = self.phase([str(self.args.binary), 'prepare-native', '--corpus', str(corpus), '--native-compressed', 'true', '--max-pod-bytes', maximum], label + '-prepare')
            else: record['preparation'] = {'status': 'reused-native-archive', 'elapsed_seconds': None}
            def inspect_native():
                value = json.loads(sidecar.read_text())
                if value['source_manifest_sha256'] != digest(corpus / 'manifest.json') or value['archive_sha256'] != digest(native) or value['archive_bytes'] != native.stat().st_size or value['pods'] != manifest['pods'] or value['compressed'] is not True:
                    raise ValueError('native archive provenance, population or encoding differs')
                return value
            native_manifest = self.inspect(inspect_native)
            write_json(self.results / (label + '-native-manifest.json'), native_manifest)
            record['storage'] = self.inspect(storage_inventory, corpus)
            representatives = ','.join(str(row['pod']) for row in cell['existing_intensity_extrema'])
            record['oracle_cold_origin'] = self.cold_origin()
            record['oracle'] = self.phase([str(self.args.binary), 'verify', '--corpus', str(corpus), '--storage-mode', 'native', '--max-pod-bytes', maximum,
                '--verify-pod-ids', representatives, '--workload-file', str(ROOT / 'bench/ac/million/workload.json')], label + '-oracle')
            # No other pilot job is alive. Faults from archive validation and
            # preload now charge the server unit, not preparation's page cache.
            record['cold_origin'] = self.cold_origin()
            try:
                record['startup'] = self.phase([str(self.args.binary), 'serve', '--corpus', str(corpus), '--auth-dir', str(self.args.auth),
                    '--storage-mode', 'native', '--workers', '16', '--max-pod-bytes', maximum, '--control-token-file', str(self.args.auth / 'preloaded-control-token')], label + '-startup', ready=True)
                startup = record['startup']
                if len(startup['readiness']) != 1 or len(startup['archive_verification']) != 1 or startup['readiness'][0]['population'] != manifest['pods'] or startup['readiness'][0]['retained_pods'] != manifest['pods']:
                    raise ValueError('startup did not retain the complete declared population')
                output = subprocess.check_output(['taskset', '-c', self.args.client_cpus, str(self.args.binary), 'drain',
                    '--control-token-file', str(self.args.auth / 'preloaded-control-token'), '--drain-timeout-seconds', '30'], text=True, timeout=40)
                drain = json.loads(output)
                CELL.validate_barrier(drain, manifest['pods'], 16, 'native')
                if any(row['processed_requests'] != 0 or row['state']['activations']['journal_entries_replayed'] != 0 for row in drain['workers']):
                    raise ValueError('preparation-only startup processed workload or retained mutations')
                record['drain'] = drain; record['after_drain_resources'] = self.snapshot(self.current)
                validate_resources(record['after_drain_resources'])
                record['status'] = 'complete'
            finally: self.stop()
        write_json(self.results / (label + '-footprint.json'), record)

    def execute(self):
        status = 'incomplete'; cleanup_error = None
        try:
            for cell in self.proposal['cells']:
                self.guard(); self.cell(cell)
            status = 'complete'
        except BaseException as error:
            status = 'incomplete'
            if self.records: self.records[-1]['error'] = str(error)
            print(json.dumps({'record_type': 'preparation-stopped', 'reason': str(error)}), flush=True)
        finally:
            try: self.stop()
            except (OSError, subprocess.SubprocessError) as error:
                cleanup_error = str(error); status = 'incomplete'
            result = {'record_type': 'native-preparation-pilot-result', 'status': status,
                'executor_source_commit': self.args.source_commit, 'proposal_sha256': self.args.proposal_sha256,
                'binary_sha256': self.args.binary_sha256, 'binary_build_source_commit': self.args.binary_source_commit,
                'source_input_equivalence': self.args.source_input_equivalence,
                'cleanup_error': cleanup_error, 'cells': self.records,
                'unattempted': [{'dataset': row['dataset'], 'model': row['model']} for row in self.proposal['cells'][len(self.records):]],
                'elapsed_seconds': time.monotonic() - self.started,
                'scope': 'preparation and complete-population startup footprint only; no timed load, SLO, capacity or main matrix conclusion'}
            try:
                with bounded_inspection(5):
                    prior_rows = json.loads((ROOT / 'research/solid-pod-scale-main.json').read_text())['corpora']
                    projections = history_projections(self.proposal, self.records, prior_rows)
                    projections['proposal_sha256'] = self.args.proposal_sha256
                    projections['executor_source_commit'] = self.args.source_commit
                    projections['binary_build_source_commit'] = self.args.binary_source_commit
                    write_json(self.results / 'preparation-projections.json', projections)
            except (OSError, ValueError, KeyError, StopIteration, TimeoutError) as error:
                result['projection_error'] = str(error)
            write_json(self.results / 'preparation-result.json', result)
        if status != 'complete': raise RuntimeError('preparation pilot incomplete; retained evidence identifies the stop')


def execute_main():
    parser = argparse.ArgumentParser()
    for key in ('proposal', 'corpora', 'auth', 'binary', 'results'): parser.add_argument('--' + key, type=Path, required=True)
    for key in ('proposal-sha256', 'source-commit', 'binary-sha256', 'binary-source-commit', 'server-cpus', 'client-cpus'): parser.add_argument('--' + key, required=True)
    parser.add_argument('--host-memory-gib', type=int, default=384)
    parser.add_argument('--outer-memory-fraction', type=float, default=.8)
    parser.add_argument('--disk-floor-gib', type=int, default=100)
    args = parser.parse_args()
    for value, length in [(args.proposal_sha256, 64), (args.binary_sha256, 64), (args.source_commit, 40), (args.binary_source_commit, 40)]:
        if not re.fullmatch('[0-9a-f]{' + str(length) + '}', value): raise ValueError('invalid explicit source identity')
    if digest(args.proposal) != args.proposal_sha256 or digest(args.binary) != args.binary_sha256:
        raise ValueError('proposal or binary digest differs')
    proposal = json.loads(args.proposal.read_text()); validate_proposal(proposal)
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    if revision != args.source_commit or subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT):
        raise ValueError('executor requires the exact clean declared source checkout')
    subprocess.run(['git', 'merge-base', '--is-ancestor', proposal['harness_source_commit'], revision], cwd=ROOT, check=True)
    args.source_input_equivalence = source_equivalence(args.binary_source_commit, revision)
    within(ROOT, DATA)
    for name in ('corpora', 'auth', 'binary'): setattr(args, name, within(getattr(args, name), DATA))
    args.results = within(args.results, RESULTS)
    if args.results.exists() and any(args.results.iterdir()): raise ValueError('results must be empty')
    servers = [int(v) for v in args.server_cpus.split(',')]; clients = [int(v) for v in args.client_cpus.split(',')]
    if len(servers) != 16 or len(clients) != 4 or len(set(servers + clients)) != 20 or min(servers + clients) < 0 or max(servers + clients) >= os.cpu_count():
        raise ValueError('require16 server and4 disjoint client CPUs on this host')
    if args.host_memory_gib != 384 or not 128 < args.host_memory_gib * args.outer_memory_fraction < args.host_memory_gib or args.disk_floor_gib < 20:
        raise ValueError('invalid candidate host outer-memory or disk reserve')
    if not Path('/sys/fs/cgroup/cgroup.controllers').exists() or not DATA.is_mount():
        raise ValueError('require dedicated data0 mount and Linux cgroup v2')
    args.corpora.mkdir(parents=True, exist_ok=True); args.results.mkdir(parents=True, exist_ok=True)
    with (DATA / 'preparation-pilot.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        outer = int(args.host_memory_gib * GIB * args.outer_memory_fraction)
        subprocess.run(['sudo', 'systemctl', 'set-property', '--runtime', 'sparq-pod-bench.slice', f'MemoryMax={outer}', 'MemorySwapMax=0'], check=True, timeout=15)
        write_json(args.results / 'preparation-input.json', {'executor_source_commit': revision, 'binary_build_source_commit': args.binary_source_commit,
            'proposal_sha256': args.proposal_sha256,
            'source_input_equivalence': args.source_input_equivalence,
            'binary_sha256': args.binary_sha256, 'executor_sha256': digest(Path(__file__)),
            'memory_scope': 'Dictionary term/record bytes are mapped. Graph/dictionary descriptors, selected metadata/caches, authorization and write overlays consume heap; cgroup anon is a retained-heap proxy, not exact allocated heap.',
            'physical_host_declaration': {'instance_type': 'r7gd.12xlarge', 'memory_gib': args.host_memory_gib, 'logical_cpus_observed': os.cpu_count()},
            'unit_allocation': {'memory_gib': 128, 'server_cpu_ids': servers, 'disjoint_client_cpu_ids': clients},
            'argv': vars(args) | {k: str(v) for k, v in vars(args).items() if isinstance(v, Path)}, 'proposal': proposal})
        pilot = Preparation(args, proposal)
        def cancelled(_signal, _frame): raise InterruptedError('preparation cancelled; stop owned units')
        signal.signal(signal.SIGTERM, cancelled); signal.signal(signal.SIGINT, cancelled)
        try:
            pilot.require_exclusive_jobs(); pilot.guard()
            if auth_requires_provision(args.auth):
                pilot.phase([str(args.binary), 'auth', '--auth-dir', str(args.auth)], 'auth-provision')
            secret = args.auth / 'preloaded-control-token'
            if not secret.exists():
                descriptor = os.open(secret, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                with os.fdopen(descriptor, 'w') as stream: stream.write(secrets.token_hex(32))
            pilot.execute()
        except BaseException as error:
            cleanup = None
            try: pilot.stop()
            except (OSError, subprocess.SubprocessError) as stopping: cleanup = str(stopping)
            if not (args.results / 'preparation-result.json').exists():
                write_json(args.results / 'preparation-result.json', {'record_type': 'native-preparation-pilot-result',
                    'status': 'incomplete', 'executor_source_commit': revision,
                    'binary_build_source_commit': args.binary_source_commit, 'binary_sha256': args.binary_sha256,
                    'proposal_sha256': args.proposal_sha256, 'source_input_equivalence': args.source_input_equivalence,
                    'setup_error': str(error), 'cleanup_error': cleanup, 'cells': [],
                    'scope': 'preparation only; no workload, SLO, capacity or main matrix admission'})
            raise


def main():
    # Includes preflight hashes; reserve30s of the reviewed3600s for cleanup.
    with bounded_inspection(3570):
        execute_main()


if __name__ == '__main__': main()
