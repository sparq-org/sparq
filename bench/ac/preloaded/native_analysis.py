"""[GPT-6] Independent native-preload accounting; no cached-campaign fallback."""
from collections import Counter
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'bench/ac/million'))
import campaign_analysis as OLD

SPEC = importlib.util.spec_from_file_location('preloaded_runner', Path(__file__).with_name('run-campaign.py'))
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)
GIB = 1024**3


class Evidence(OLD.Evidence):
    """The native executor closes/compresses both structured JSONL and text logs."""
    def locate(self, name):
        paths = [self.safe(name)]
        if name.endswith(('.jsonl', '.log')): paths.append(self.safe(name + '.zst'))
        present = [path for path in paths if path.is_file()]
        if len(present) > 1:
            self.errors.append({'file': name, 'error': 'ambiguous-plain-and-compressed-input'})
            return None
        return present[0] if present else None


def source_bindings(campaign, root=ROOT):
    errors = []; verified = {}; shapes = []
    source = campaign.get('source_commit', '')
    if not re.fullmatch('[0-9a-f]{40}', source): return {'passed': False, 'issues': ['invalid-source-commit'], 'verified': {}}
    if not RUNNER.REQUIRED_BINDINGS <= campaign.get('bindings', {}).keys(): errors.append('required-source-bindings-missing')
    for name, digest in campaign.get('bindings', {}).items():
        if Path(name).is_absolute() or '..' in Path(name).parts:
            errors.append('unsafe-source-path'); continue
        try:
            content = subprocess.check_output(['git', 'show', source + ':' + name], cwd=root, stderr=subprocess.PIPE, timeout=30)
            actual = hashlib.sha256(content).hexdigest()
            if actual != digest: errors.append('source-binding-differs:' + name)
            else:
                verified[name] = actual
                if name == 'bench/ac/million/campaign-20260906.json': shapes = json.loads(content)['corpora']
        except (OSError, subprocess.SubprocessError): errors.append('source-binding-unavailable:' + name)
    for name in ('bench/ac/preloaded/run-cell.py', 'bench/ac/million/run-campaign.py'):
        try:
            content = subprocess.check_output(['git', 'show', source + ':' + name], cwd=root, stderr=subprocess.PIPE, timeout=30)
            verified[name] = hashlib.sha256(content).hexdigest()
        except (OSError, subprocess.SubprocessError): errors.append('runtime-source-unavailable:' + name)
    return {'passed': not errors, 'issues': errors, 'verified': verified, 'corpus_shape_definitions': shapes,
            'scope': 'Compare each declared input to its blob in the exact locally available measured Git revision; no working-tree or latest-source substitution.'}


def scalar(text):
    return int(text.strip()) if isinstance(text, str) and text.strip().isdigit() else None


def cpus(text):
    values = []
    try:
        for part in text.split(','):
            ends = part.split('-'); first = int(ends[0]); last = int(ends[-1])
            if len(ends) > 2 or first < 0 or last < first or last - first > 4096: return None
            values.extend(range(first, last + 1))
        return sorted(set(values))
    except (AttributeError, ValueError): return None


def io_devices(text):
    result = {}
    try:
        for line in (text or '').splitlines():
            fields = line.split(); result[fields[0]] = {key: int(value) for key, value in (field.split('=') for field in fields[1:])}
    except (IndexError, ValueError): return None
    return result


def native_snapshot(document):
    document = document or {}; group = document.get('cgroup', {}); process = document.get('process', {})
    status = process.get('status', ''); affinity = next((line.split(':', 1)[1].strip() for line in status.splitlines() if line.startswith('Cpus_allowed_list:')), None)
    faults = None
    try:
        fields = process['stat'].rsplit(')', 1)[1].split()
        faults = {'minor': int(fields[7]), 'major': int(fields[9])}
    except (KeyError, ValueError, IndexError): pass
    return {'memory_peak_bytes': scalar(group.get('memory.peak')), 'memory_current_bytes': scalar(group.get('memory.current')),
        'ceiling_bytes': scalar(group.get('memory.max')), 'swap_ceiling_bytes': scalar(group.get('memory.swap.max')),
        'memory_events': OLD.key_values(group.get('memory.events')), 'memory_stat': OLD.key_values(group.get('memory.stat')),
        'cpu_stat': OLD.key_values(group.get('cpu.stat')), 'io_devices': io_devices(group.get('io.stat')),
        'process_status_bytes': {key: value for key, value in OLD.key_values(status).items() if key.startswith('Vm')},
        'process_smaps_bytes': OLD.key_values(process.get('smaps_rollup')), 'process_io_bytes': OLD.key_values(process.get('io')),
        'process_faults': faults, 'server_cpu_ids_observed': cpus(affinity),
        'open_descriptors': process.get('open_descriptors'), 'unit': document.get('systemd', {}).get('ControlGroup'),
        'host_meminfo_bytes': OLD.key_values(document.get('host', {}).get('meminfo')),
        'host_memory_pressure': document.get('host', {}).get('pressure/memory'),
        'study_slice_current_bytes': scalar(document.get('study_slice', {}).get('memory.current')),
        'study_slice_peak_bytes': scalar(document.get('study_slice', {}).get('memory.peak')),
        'errors': document.get('errors', []), 'host_errors': document.get('host_errors', [])}


def resources(evidence, prefix, memory, host):
    documents = {phase: evidence.document(prefix + '-' + phase + '-native-accounting.json') for phase in ('preload', 'before', 'after')}
    samples = {phase: native_snapshot(document) for phase, document in documents.items()}
    issues = []; guard_failures = []
    for phase, sample in samples.items():
        if not documents[phase] or sample['errors']: issues.append(phase + '-native-accounting-missing-or-failed')
        if sample['ceiling_bytes'] != memory * GIB or sample['swap_ceiling_bytes'] != 0: issues.append(phase + '-actual-memory-limit-differs')
        if sample['server_cpu_ids_observed'] != sorted(host['server_cpu_pool']): issues.append(phase + '-actual-cpu-affinity-differs')
        if not sample['unit'] or 'sparq-pod-bench.slice' not in sample['unit'].split('/'): issues.append(phase + '-unbound-cgroup')
        if sample['memory_peak_bytes'] is None or sample['memory_current_bytes'] is None or not {'oom', 'oom_kill'} <= sample['memory_events'].keys(): issues.append(phase + '-required-memory-counters-missing')
        if not {'anon', 'file'} <= sample['memory_stat'].keys() or 'usage_usec' not in sample['cpu_stat']: issues.append(phase + '-memory-class-or-cpu-counter-missing')
        if sample['memory_peak_bytes'] is not None and sample['memory_peak_bytes'] > memory * GIB: guard_failures.append(phase + '-memory-ceiling-exceeded')
        if sample['memory_events'].get('oom', 0) or sample['memory_events'].get('oom_kill', 0): guard_failures.append(phase + '-oom-observed')
    if len({sample['unit'] for sample in samples.values()}) != 1: issues.append('server-cgroup-changed')
    before, after = samples['before'], samples['after']; deltas = {}
    for section in ('cpu_stat', 'memory_events', 'process_io_bytes'):
        deltas[section] = {key: after[section][key] - value for key, value in before[section].items() if key in after[section]}
        if any(value < 0 for value in deltas[section].values()): issues.append('cumulative-counter-regressed:' + section)
    paging = {}
    for key in ('pgfault', 'pgmajfault', 'workingset_refault_file', 'pgscan', 'pgsteal'):
        b = before['memory_stat'].get(key); a = after['memory_stat'].get(key)
        paging[key] = a - b if a is not None and b is not None and a >= b else None
    reads = None
    if before['io_devices'] is not None and after['io_devices'] is not None:
        values = [fields.get('rbytes', 0) - before['io_devices'].get(device, {}).get('rbytes', 0) for device, fields in after['io_devices'].items()]
        reads = sum(values) if all(value >= 0 for value in values) else None
    paging['cgroup_device_read_bytes_delta'] = reads
    paging['positive_refault_and_major_fault_and_read_io'] = all(value is not None and value > 0 for value in (paging['workingset_refault_file'], paging['pgmajfault'], reads))
    paging['scope'] = 'Measured counters between complete worker drains. Positive indicators support file working-set paging; they do not attribute every I/O byte to the archive or prove all heap state spills. Zero/missing counters are not proof of residency.'
    return {'passed': not issues and not guard_failures, 'issues': issues, 'guard_failures': guard_failures,
        'samples': samples, 'deltas': deltas, 'paging': paging,
        'physical_host': {'instance_type': host['instance_type'], 'memory_gib': host['physical_memory_gib'], 'vcpus': host['physical_vcpus']},
        'server_memory_gib': memory, 'fixed_server_cpu_ids': host['server_cpu_pool'], 'disjoint_client_cpu_ids': host['client_cpu_pool'],
        'scope': 'Server cgroup peak includes startup/warmup/charged file pages. Anonymous memory is a heap proxy; file pages are separate. CPU delta spans the client window and complete post-client worker drain. Endpoint host headroom is not a full-host peak measurement or proof of a smaller physical-machine fit.'}


def preload(evidence, prefix, population, workers, native_manifest):
    issues = []; rows = list(evidence.rows(prefix + '-server.log', strict=False))
    ready = [row for row in rows if row.get('record_type') == 'all-population-ready']
    verified = [row for row in rows if row.get('record_type') == 'native-archive-verified']
    start = evidence.document(prefix + '-startup-boundary.json')
    before = evidence.document(prefix + '-before-drain.jsonl'); after = evidence.document(prefix + '-after-drain.jsonl')
    if len(ready) != 1 or len(verified) != 1: issues.append('missing-or-duplicate-population-ready-or-archive-verification')
    record = ready[0] if len(ready) == 1 else {}
    if verified and verified[0].get('manifest') != native_manifest: issues.append('startup-archive-identity-differs')
    if record.get('archive_verification') != (verified[0] if len(verified) == 1 else None): issues.append('ready-archive-binding-differs')
    reports = record.get('worker_reports', [])
    if record.get('population') != population or record.get('retained_pods') != population or len(reports) != workers:
        issues.append('readiness-does-not-cover-complete-population')
    else:
        try:
            reports = sorted(reports, key=lambda row: row['worker'])
            if [row['worker'] for row in reports] != list(range(workers)): raise ValueError('worker identity order differs')
            initial = {'record_type': 'worker-drain-complete', 'passed': True, 'workers': [{'processed_requests': 0, 'state': row['state']} for row in reports]}
            RUNNER.CELL.validate_barrier(initial, population, workers, 'native')
            for row in reports:
                if row['assigned_pods'] != row['state']['retained_pods'] or row['state']['activations']['journal_entries_replayed'] != 0: raise ValueError('startup journal or ownership differs')
            if sum(row['state']['retained_source_bytes'] for row in reports) != native_manifest['source_bytes']: raise ValueError('retained source volume differs')
            RUNNER.CELL.validate_barrier(before, population, workers, 'native', initial)
            RUNNER.CELL.validate_barrier(after, population, workers, 'native', before)
        except (AttributeError, KeyError, TypeError, ValueError) as error: issues.append('population-or-drain-invariant:' + str(error))
    settings = record.get('settings', {})
    if settings.get('storage-mode') != 'native' or settings.get('workers') != str(workers): issues.append('server-mode-or-worker-setting-differs')
    if not start or start.get('complete') is not True or start.get('cold_os_caches_before_launch') is not True or not OLD.numeric(start.get('elapsed_seconds')):
        issues.append('complete-cold-startup-boundary-missing')
    return {'passed': not issues, 'issues': issues, 'complete_startup_seconds': start.get('elapsed_seconds') if start else None,
            'startup_boundary': start, 'readiness': record or None,
            'worker_preload_us': [row.get('elapsed_us') for row in reports], 'before_drain': before, 'after_drain': after,
            'scope': 'Every native dataset and initial authorization remains retained. Counter invariants prohibit timed parsing, reopening, initial materialization or journal replay; session caches and failed-mutation rollback remain separate work.'}


def population(evidence, dataset, model, completion):
    label = dataset['id'] + '-' + model
    record = evidence.document(label + '-population.json') or {}
    manifest = evidence.document(label + '-source-manifest.json') or {}
    native = record.get('native_manifest', {}); issues = []; storage_issues = []
    if record.get('status') != 'complete' or completion != 'complete': issues.append('population-preparation-incomplete')
    if record.get('manifest') != manifest: storage_issues.append('retained-source-manifest-object-differs')
    source_path = evidence.locate(label + '-source-manifest.json')
    source_sha = OLD.sha_file(source_path) if source_path else None
    if not source_sha or source_sha != record.get('source_manifest_sha256') or source_sha != native.get('source_manifest_sha256'):
        storage_issues.append('native-source-manifest-bytes-unbound')
    if manifest.get('config') != dataset['expected_config'] or RUNNER.canonical_digest(dataset['expected_config']) != dataset['expected_config_sha256']:
        storage_issues.append('frozen-complete-service-shape-disagrees')
    if manifest.get('pods') != dataset['pods'] or manifest.get('model') != model or manifest.get('populated') is not True or manifest.get('binary_payloads_included') is not False:
        storage_issues.append('complete-metadata-population-or-model-disagrees')
    totals = Counter(); services = Counter(); extrema = {}; count = 0; maximum = 0
    classes = {(row['numerator'], row['denominator']) for row in dataset['expected_config'].get('volume_classes', [])}
    for row in evidence.rows(label + '-pod-summaries.jsonl'):
        if row.get('pod_id') != count: storage_issues.append('inventory-pod-sequence-disagrees')
        count += 1
        if any(type(row.get(key)) is not int or row[key] <= 0 for key in ('records', 'quads', 'bytes', 'compressed_bytes', 'intensity_numerator', 'intensity_denominator')):
            storage_issues.append('invalid-inventory-values'); continue
        for key in ('records', 'quads', 'bytes', 'compressed_bytes'): totals[key] += row[key]
        if not isinstance(row.get('records_by_service'), dict) or any(type(v) is not int or v < 0 for v in row['records_by_service'].values()):
            storage_issues.append('invalid-service-inventory'); continue
        services.update(row['records_by_service']); maximum = max(maximum, row['bytes'])
        key = (row['intensity_numerator'], row['intensity_denominator'])
        if key not in classes: storage_issues.append('unconfigured-intensity-class')
        candidate = {'pod': row['pod_id'], 'source_bytes': row['bytes'], 'intensity_numerator': key[0], 'intensity_denominator': key[1]}
        low, high = extrema.get(key, (candidate, candidate))
        extrema[key] = (min((low, candidate), key=lambda r: (r['source_bytes'], r['pod'])), min((high, candidate), key=lambda r: (-r['source_bytes'], r['pod'])))
    expected = {'pods': count, 'records': totals['records'], 'quads': totals['quads'], 'source_bytes': totals['bytes'], 'packed_bytes': totals['compressed_bytes'], 'index_bytes': count * 24, 'maximum_pod_source_bytes': maximum}
    if count != dataset['pods'] or any(type(manifest.get(key)) is not int or manifest[key] != value for key, value in expected.items()): storage_issues.append('complete-inventory-totals-disagree')
    if sum(services.values()) != totals['records']: storage_issues.append('service-record-total-disagrees')
    selected = {row['pod']: row for pair in extrema.values() for row in pair}; representatives = [selected[key] for key in sorted(selected)]
    if not representatives or record.get('representatives') != representatives: issues.append('representatives-not-full-intensity-extrema')
    inventory = record.get('storage', {}); entries = inventory.get('entries', []); by_path = {row.get('path'): row for row in entries}
    if len(by_path) != len(entries): storage_issues.append('duplicate-storage-entry')
    allocated = 0; file_bytes = 0; files = directories = 0
    for entry in entries:
        name = entry.get('path', '')
        if not name or Path(name).is_absolute() or '..' in Path(name).parts or entry.get('kind') not in ('file', 'directory') or any(type(entry.get(key)) is not int or entry[key] < 0 for key in ('logical_bytes', 'allocated_bytes')):
            storage_issues.append('invalid-physical-storage-entry'); continue
        allocated += entry['allocated_bytes']
        if entry['kind'] == 'file': files += 1; file_bytes += entry['logical_bytes']
        else: directories += 1
    whole = inventory.get('allocated_bytes_including_directories')
    if inventory.get('files') != files or inventory.get('directories_including_root') != directories + 1 or inventory.get('file_logical_bytes') != file_bytes or type(whole) is not int or whole < allocated or (whole - allocated) % 512:
        storage_issues.append('physical-storage-inventory-arithmetic-disagrees')
    for name, expected_bytes in [('pods.nqpack', manifest.get('packed_bytes')), ('pods.index', manifest.get('index_bytes')), ('pods.native', native.get('archive_bytes'))]:
        entry = by_path.get(name, {})
        if entry.get('kind') != 'file' or entry.get('logical_bytes') != expected_bytes or type(entry.get('allocated_bytes')) is not int: storage_issues.append('storage-file-accounting-disagrees:' + name)
    if native.get('format') != 'sparq-population-native-v1' or native.get('pods') != dataset['pods'] or native.get('source_bytes') != manifest.get('source_bytes') or native.get('compressed') is not True or not re.fullmatch('[0-9a-f]{64}', native.get('archive_sha256', '')):
        storage_issues.append('native-archive-identity-or-encoding-invalid')
    oracle = evidence.document(label + '-oracle.json') or {}
    rows = list(evidence.rows(label + '-oracle.log', strict=False))
    starts = [row.get('pod') for row in rows if row.get('record_type') == 'verify-pod-start']
    ends = [row for row in rows if row.get('record_type') == 'verification-complete']
    if starts != [row['pod'] for row in representatives] or len(ends) != 1 or ends[0].get('sampled_pods') != len(representatives) or ends[0].get('checks') != 108 * len(representatives):
        issues.append('representative-oracle-completion-disagrees')
    if oracle.get('status') != 'complete' or record.get('oracle') != oracle: issues.append('representative-unit-not-complete')
    try: RUNNER.PREP.validate_resources(oracle.get('resources', {}))
    except (KeyError, TypeError, ValueError): issues.append('representative-resource-evidence-not-admitted')
    for name in ('generation', 'native_preparation'):
        phase = record.get(name, {})
        if phase.get('status') == 'complete':
            try: RUNNER.PREP.validate_resources(phase.get('resources', {}))
            except (KeyError, TypeError, ValueError): issues.append(name + '-resource-evidence-not-admitted')
        elif phase.get('status') != 'reused' or phase.get('new_preparation_time') is not None:
            issues.append(name + '-completion-or-explicit-reuse-missing')
    issues.extend(storage_issues)
    return {'dataset': dataset['id'], 'model': model, 'role': dataset['role'], 'complete': not issues,
        'issues': sorted(set(issues)), 'storage_inventory_consistent': not storage_issues,
        'manifest': manifest or None, 'native_manifest': native or None, 'source_manifest_sha256': source_sha,
        'records_by_service': dict(services), 'storage': inventory or None,
        'native_logical_bytes': native.get('archive_bytes'), 'native_allocated_bytes': by_path.get('pods.native', {}).get('allocated_bytes'),
        'whole_corpus_allocated_bytes': inventory.get('allocated_bytes_including_directories'),
        'preparation': {'generation': record.get('generation'), 'native': record.get('native_preparation'), 'oracle': oracle or None},
        'verification': {'representatives': representatives, 'checks': ends[0].get('checks') if len(ends) == 1 else None},
        'scope': 'Full retrieved inventory reconciled; source-side pack/index hash and index-entry validation recorded by the reviewed executor. Exact source-manifest bytes and native digest are rebound at startup; large payload archives are not copied locally for rehashing.'}


def runtime_issues(evidence, directory, campaign):
    issues = []
    runtime = evidence.document(directory + '/preloaded-runtime.json') or {}
    for key in ('source_commit', 'binary_sha256'):
        if runtime.get(key) != campaign.get(key): issues.append('cell-runtime-binding-disagrees:' + key)
    if runtime.get('campaign_sha256') != campaign['_sha256']: issues.append('cell-campaign-hash-disagrees')
    for key, path in [('runner_sha256', 'bench/ac/preloaded/run-cell.py'), ('base_runner_sha256', 'bench/ac/million/run-campaign.py'), ('workload_sha256', 'bench/ac/million/workload.json')]:
        if not campaign['_source_bindings'].get(path) or runtime.get(key) != campaign['_source_bindings'][path]: issues.append('cell-runtime-source-disagrees:' + key)
    return issues


def admission_attempt(evidence, campaign, metadata, declared, pop, prefix, result):
    issues = runtime_issues(evidence, declared['directory'], campaign)
    record = evidence.document(prefix + '-preload-admission.json') or {}
    terminal = evidence.document(prefix + '-preload-terminal.json')
    boundary = evidence.document(prefix + '-startup-boundary.json') or {}
    if record != declared.get('admission'): issues.append('completion-admission-record-disagrees')
    if terminal != record.get('terminal_capture'): issues.append('terminal-cgroup-record-disagrees')
    if record.get('population') != metadata['population'] or record.get('memory_max_bytes') != metadata['memory_gib'] * GIB or record.get('startup_timeout_seconds') != campaign['preloaded']['startup_timeout_seconds']:
        issues.append('admission-population-limit-or-deadline-disagrees')
    if record.get('archive_manifest') != pop.get('native_manifest'): issues.append('admission-archive-binding-disagrees')
    if not pop['complete']: issues.append('complete-population-preparation-not-admitted')
    try: RUNNER.CELL.ADMISSION.classify(record)
    except (KeyError, TypeError, ValueError) as error: issues.append('unsubstantiated-admission-failure:' + str(error))
    log = evidence.text(prefix + '-server.log')
    if log is None or hashlib.sha256(log.encode()).hexdigest() != record.get('server_log_sha256'):
        issues.append('admission-startup-log-binding-disagrees')
    try:
        rows = [json.loads(line) for line in (log or '').splitlines()]
        RUNNER.CELL.ADMISSION.validate_records(rows, metadata['population'], 16, pop.get('native_manifest'))
    except (KeyError, TypeError, ValueError) as error: issues.append('invalid-partial-startup-output:' + str(error))
    if boundary.get('complete') is not False or boundary.get('unit') != record.get('unit') or boundary.get('cold_os_caches_before_launch') is not True or not OLD.numeric(boundary.get('elapsed_seconds')) or not OLD.numeric(record.get('startup_elapsed_seconds')) or boundary.get('elapsed_seconds', -1) < record.get('startup_elapsed_seconds', 0):
        issues.append('failed-startup-boundary-not-bound')
    for suffix in ('requests.jsonl', 'warmup.jsonl', 'audit.jsonl', 'summary.json'):
        if evidence.locate(prefix + '-' + suffix) is not None: issues.append('workload-output-present-for-failed-admission:' + suffix)
    events = list(evidence.rows(declared['directory'] + '/campaign-events.jsonl'))
    matches = [row for row in events if row.get('record_type') == 'preload-admission-failed']
    if len(matches) != 1 or any(matches[0].get(key) != record.get(key) for key in ('outcome', 'unit', 'population')) or matches[0].get('artifact') != Path(prefix).name + '-preload-admission.json':
        issues.append('canonical-admission-event-disagrees')
    result.update(local_guard='admission-failed' if not issues else 'inconclusive', issues=sorted(set(issues)),
        valid_for_admission_inference=not issues, admission=record,
        resources={'failure_snapshot': native_snapshot(record.get('resources')), 'terminal_cgroup': terminal,
                   'physical_host': campaign['host'], 'server_memory_gib': metadata['memory_gib'],
                   'scope': 'Startup and terminal collector counters only; no measured request-window deltas or paging inference. ExecStopPost adds a small collector process after the server exits.'},
        preload={'complete': False, 'startup_boundary': boundary})
    return result


def analyze_cell(evidence, campaign, metadata, declared, pop, scratch):
    issues = []; label = declared.get('label'); directory = declared.get('directory')
    result = {**metadata, 'execution_status': declared.get('status', 'unattempted'), 'valid_for_inference': False,
              'valid_for_admission_inference': False, 'local_guard': 'unmeasured', 'issues': [], 'requests': None, 'resources': None, 'preload': None}
    if declared.get('status') == 'unattempted':
        result['issues'] = ['cell-not-complete']; result['execution_error'] = declared.get('error')
        return result
    if declared.get('status') not in ('complete', 'admission-failed'): issues.append('cell-not-complete')
    if not isinstance(directory, str) or not re.fullmatch(r'cell-[0-9]{5}', directory) or label != metadata['label']:
        result.update(local_guard='inconclusive', issues=['recursive-cell-location-or-label-disagrees']); return result
    prefix = directory + '/' + label
    if declared.get('status') == 'admission-failed':
        return admission_attempt(evidence, campaign, metadata, declared, pop, prefix, result)
    reported = evidence.document(prefix + '-summary.json') or {}
    reconciliation = evidence.document(prefix + '-reconciliation.json') or {}
    if declared.get('summary') != reported: issues.append('completion-summary-differs-from-cell-file')
    issues.extend(runtime_issues(evidence, directory, campaign))
    for key in ('dataset', 'group', 'memory_gib', 'cpus', 'rate_override', 'replicate', 'model', 'seed'):
        if reported.get(key) != metadata[key]: issues.append('runner-metadata-disagrees:' + key)
    for key in ('load_exit_code', 'warmup_exit_code', 'audit_exit_code'):
        if reported.get(key) != 0: issues.append(key + '-not-successful')
    if reconciliation.get('passed') is not True: issues.append('reported-reconciliation-not-passed')
    analyzer = OLD.RequestAnalysis(scratch, campaign['measurement'])
    try:
        analyzer.audit(evidence.rows(prefix + '-audit.jsonl'))
        analyzer.requests(evidence.rows(prefix + '-requests.jsonl'))
        requests = analyzer.result()
    finally:
        analyzer.close(); scratch.unlink(missing_ok=True)
    issues.extend(requests['issues']); issues.extend(requests['mutation']['issues'])
    if reconciliation.get('passed') is not True or reported.get('audit_exit_code') != 0:
        requests['mutation'].update(reconciled=False, unknown_resolved_committed=None, unknown_resolved_not_committed=None)
    if not requests['mutation']['reconciled']: issues.append('mutation-reconciliation-not-admitted')
    group = next(row for row in campaign['groups'] if row['id'] == metadata['group'])
    settings = requests['load_metadata']['settings']; m = campaign['measurement']
    expected = {'seed': str(metadata['seed']), 'scenario': group['scenario'], 'selection': group['selection'], 'mix': 'journeys',
        'timeout-ms': str(m['request_timeout_ms']), 'max-inflight': str(m['maximum_inflight']),
        'duration-seconds': str(m['measurement_seconds']), 'requests': str(m['minimum_offered']),
        'rate': None if metadata['rate_override'] == 'derived' else str(metadata['rate_override'])}
    for key, value in expected.items():
        if settings.get(key) != value: issues.append('raw-setting-disagrees:' + key)
    actual_rate = requests['offered_rate']; expected_rate = requests['load_metadata']['derived_rate'] if metadata['rate_override'] == 'derived' else metadata['rate_override']
    if not OLD.numeric(actual_rate) or not OLD.numeric(expected_rate) or actual_rate <= 0 or not math.isclose(actual_rate, expected_rate, rel_tol=1e-12): issues.append('raw-offered-rate-disagrees')
    if reported.get('offered_rate') != actual_rate: issues.append('reported-rate-disagrees')
    if requests['load_metadata']['workload_sha256'] != campaign['bindings'].get('bench/ac/million/workload.json'): issues.append('raw-workload-hash-disagrees')
    for key in ('pods', 'packed_sha256', 'index_sha256'):
        if requests['load_metadata']['corpus'][key] != (pop.get('manifest') or {}).get(key): issues.append('raw-population-binding-disagrees:' + key)
    state = preload(evidence, prefix, metadata['population'], 16, pop.get('native_manifest'))
    accounting = resources(evidence, prefix, metadata['memory_gib'], campaign['host'])
    ready_settings = (state['readiness'] or {}).get('settings', {})
    for key, value in {'max-pod-bytes': str(group['max_pod_bytes']), 'queue-capacity': str(m['queue_capacity_per_worker'])}.items():
        if ready_settings.get(key) != value: issues.append('server-setting-disagrees:' + key)
    unit_path = accounting['samples']['preload']['unit']
    if not unit_path or (state['startup_boundary'] or {}).get('unit') != unit_path.rsplit('/', 1)[-1]: issues.append('startup-boundary-cgroup-differs')
    issues.extend(state['issues']); issues.extend(accounting['issues'])
    if not pop['complete']: issues.append('complete-population-preparation-not-admitted')
    if requests['client_limited']: issues.append('client-limited')
    if requests['outcomes'].get('client-plan-exhausted', 0): issues.append('workload-plan-exhausted')
    valid = not issues
    passes = valid and accounting['passed'] and requests['queue']['passed'] and requests['success_fraction_of_offered'] >= m['success_fraction'] and requests['deadline_fraction_of_offered'] >= m['deadline_fraction_of_all_offered'] and requests['within_server_production_deadline'] / requests['offered'] >= m['deadline_fraction_of_all_offered']
    independent = {'offered': requests['offered'], 'successful': requests['successful'],
        'within_local_deadline': requests['within_scheduled_deadline'], 'outcomes': requests['outcomes'],
        'correctness_passed': requests['mutation']['reconciled'], 'resource_guard_passed': accounting['passed'],
        'queue_stability_passed': requests['queue']['passed'], 'passes_local_guard': passes}
    disagreements = [{'field': key, 'recorded': reported[key], 'independent': value} for key, value in independent.items() if key in reported and reported[key] != value]
    if disagreements:
        issues.append('recorded-versus-independent-accounting-disagreement'); valid = False; passes = False
    result.update(valid_for_inference=valid, local_guard='pass' if passes else 'fail' if valid else 'inconclusive',
                  issues=sorted(set(issues)), requests=requests, resources=accounting, preload=state,
                  reported_guard_disagreements=disagreements,
                  guard_comparison_scope='Independent admission includes additional native source/preload/affinity checks; recorded counters and guards are retained for review and never override recomputed evidence.')
    return result


def retained_history_admission(campaign, cells, binding):
    """Describe observed central-history startup admission, not service capacity."""
    shapes = {row['id']: row for row in binding.get('corpus_shape_definitions', [])}
    eligible = set()
    for dataset in campaign['corpora']:
        shape = shapes.get(dataset.get('shape_reference'), {})
        if (binding.get('passed') is True and type(dataset.get('pods')) is int and dataset['pods'] >= 1_000_000
                and shape.get('profile') == 'history' and shape.get('role') == 'partially-calibrated-retained-history-scenario'
                and all(dataset.get(key) == shape.get(key) for key in ('profile', 'role', 'models', 'config_file'))):
            eligible.add(dataset['id'])
    attempts = []
    for cell in cells:
        if cell['dataset'] not in eligible: continue
        ready = (cell.get('preload') or {}).get('readiness') or {}
        if cell.get('execution_status') == 'unattempted': state = 'unmeasured'
        elif (cell.get('valid_for_inference') is True and (cell.get('preload') or {}).get('passed') is True
              and ready.get('population') == cell['population'] == ready.get('retained_pods')):
            state = 'admitted'
        elif cell.get('execution_status') == 'admission-failed' and cell.get('valid_for_admission_inference') is True:
            state = 'failed'
        else: state = 'inconclusive'
        attempts.append({key: cell[key] for key in ('label', 'dataset', 'model', 'population', 'memory_gib', 'cpus', 'group', 'replicate')} |
            {'state': state, 'admission_failure': (cell.get('admission') or {}).get('outcome') if state == 'failed' else None,
             'response_guard': cell.get('local_guard') if cell.get('valid_for_inference') else None})
    states = {}
    for model in ('wac', 'acp'):
        rows = [row for row in attempts if row['model'] == model]
        observed = {row['state'] for row in rows}
        state = next((value for value in ('admitted', 'inconclusive', 'failed') if value in observed), 'unmeasured')
        states[model] = {'state': state, 'counts': {value: sum(row['state'] == value for row in rows) for value in ('admitted', 'failed', 'unmeasured', 'inconclusive')}}
    return {'minimum_population': 1_000_000, 'eligible_datasets': sorted(eligible), 'by_model': states, 'attempts': attempts,
        'scope': 'Central partially calibrated retained-history shapes only, matched to source-bound profile/role/config references. Each model state says whether any validated attempt admitted that complete population; failed tiers and unattempted cells remain listed. Models may admit at different resources. Startup admission does not establish responsive-service capacity, physical-machine fit at the cgroup limit, or a representative all-service joint distribution. Compact controls and separately named stress profiles are excluded.'}


def analyze(root, review_path=None, scratch_root=None, source_root=ROOT):
    evidence = Evidence(root)
    campaign = evidence.document('frozen-campaign.json')
    completion = evidence.document('campaign-result.json')
    if not campaign or not completion or campaign.get('status') != 'frozen-before-measurement' or not campaign.get('campaign_id', '').startswith('solid-pod-preloaded-'):
        raise ValueError('native frozen campaign and source-bound result required; no old-campaign fallback')
    frozen_path = evidence.locate('frozen-campaign.json'); campaign_hash = OLD.sha_file(frozen_path)
    binding = source_bindings(campaign, source_root); global_issues = list(binding['issues'])
    event_files = {name.removesuffix('.zst') for name in evidence.manifest_hashes if name.endswith(('campaign-events.jsonl', 'campaign-events.jsonl.zst'))}
    events = [dict(row, artifact=path) for path in sorted(event_files) for row in evidence.rows(path)]
    quarantines = [row for row in events if row.get('record_type') == 'correctness-quarantine']
    if campaign.get('source_commit') != campaign.get('binary_build_source_commit'): global_issues.append('binary-source-revision-differs')
    for key in ('source_commit', 'binary_build_source_commit', 'binary_sha256'):
        if completion.get(key) != campaign.get(key): global_issues.append('completion-identity-disagrees:' + key)
    if completion.get('campaign_sha256') != campaign_hash: global_issues.append('completion-campaign-hash-disagrees')
    if completion.get('record_type') != 'native-preloaded-campaign-result' or completion.get('status') != 'complete': global_issues.append('whole-campaign-incomplete')
    if completion.get('cleanup_errors') or completion.get('finalization_error') or completion.get('error'): global_issues.append('campaign-cleanup-or-execution-error')
    if completion.get('source_quarantined') is not False: global_issues.append('source-quarantined-or-quarantine-status-missing')
    if campaign.get('preloaded', {}).get('admission_outcomes_version') != 1 or any(campaign.get('stop_rules', {}).get(key) != value for key, value in RUNNER.STOP_BEHAVIOR.items()):
        global_issues.append('frozen-preload-admission-stop-rules-missing-or-different')
    if evidence.manifest_status != 'verified': global_issues.append('final-manifest-not-verified')
    review = json.loads(Path(review_path).read_text()) if review_path else {}
    review_matches = all(review.get(key) == expected for key, expected in {
        'source_commit': campaign.get('source_commit'), 'binary_sha256': campaign.get('binary_sha256'),
        'campaign_sha256': campaign_hash, 'manifest_sha256': evidence.hashes.get('MANIFEST.sha256')}.items())
    review_status = review.get('status') if review_matches and review.get('status') in ('passed', 'quarantined') else 'unreviewed'
    if completion.get('source_quarantined') is True or quarantines:
        review_status = 'quarantined'; global_issues.append('canonical-source-quarantine')
    if review_status != 'passed': global_issues.append('source-review-' + review_status)
    planned_count = sum(len(group['datasets']) * len(group['memory_gib']) * len(group.get('rates', ['derived'])) * group['repeat'] * 2 for group in campaign['groups'])
    if not 0 < planned_count <= campaign['execution']['maximum_cells'] <= 10000: raise ValueError('declared plan exceeds bounded analysis envelope')
    planned = RUNNER.plan_cells(campaign)
    declared = completion.get('cells', [])
    if len(declared) != len(planned) or any(row.get('selection') != selection for row, selection in zip(declared, planned)):
        global_issues.append('planned-cell-order-or-completeness-disagrees')
    host = campaign['host']
    if host.get('server_cpu_pool') is None or len(host['server_cpu_pool']) != 16 or len(host.get('client_cpu_pool', [])) != 4 or len(set(host['server_cpu_pool'] + host['client_cpu_pool'])) != 20:
        global_issues.append('fixed-disjoint-cpu-allocation-invalid')
    if host.get('physical_memory_gib') != 384 or host.get('physical_vcpus') != 48 or host.get('instance_type') != 'r7gd.12xlarge' or host.get('swap') is not False:
        global_issues.append('declared-physical-host-differs')
    if any(group.get('storage_mode') != 'native' or group.get('cpus') != [16] for group in campaign['groups']): global_issues.append('cached-or-changed-cpu-cell-not-native-main')
    if not campaign['seeds'] or any(type(value) is not int or not 0 <= value < 2**64 for value in campaign['seeds']) or len(set(campaign['seeds'])) != len(campaign['seeds']): global_issues.append('independent-seeds-invalid-or-duplicated')
    population_records = completion.get('populations', []); inventories = {}
    for dataset in campaign['corpora']:
        for model in dataset['models']:
            matches = [row for row in population_records if row.get('dataset') == dataset['id'] and row.get('model') == model]
            prepared = matches[0] if len(matches) == 1 else {}
            inventories[dataset['id'], model] = population(evidence, dataset, model, prepared.get('status'))
            saved = evidence.document(dataset['id'] + '-' + model + '-population.json')
            if prepared != saved:
                inventories[dataset['id'], model]['issues'].append('completion-population-record-disagrees')
                inventories[dataset['id'], model]['complete'] = False
    campaign['_sha256'] = campaign_hash; campaign['_source_bindings'] = binding['verified']
    cells = []
    with tempfile.TemporaryDirectory(prefix='sparq-native-analysis-', dir=scratch_root) as scratch:
        for index, selection in enumerate(planned):
            group = next(row for row in campaign['groups'] if row['id'] == selection['group'])
            dataset = next(row for row in campaign['corpora'] if row['id'] == selection['dataset'])
            rate = 'derived' if selection['rate'] == 'derived' else float(selection['rate'])
            label = f'{selection["dataset"]}-{selection["group"]}-ram{selection["memory_gib"]}-cpu16-r{rate}-{selection["replicate"]}-{selection["model"]}'
            metadata = {**selection, 'label': label, 'cpus': 16, 'rate_override': rate,
                'population': dataset['pods'], 'seed': campaign['seeds'][selection['replicate']],
                'required_replicates': group['repeat'], 'corpus_role': dataset['role']}
            row = declared[index] if index < len(declared) else {'status': 'unattempted'}
            try: cell = analyze_cell(evidence, campaign, metadata, row, inventories[dataset['id'], selection['model']], Path(scratch) / f'{index}.sqlite')
            except (KeyError, TypeError, ValueError, OSError) as error:
                cell = {**metadata, 'execution_status': row.get('status'), 'local_guard': 'inconclusive', 'valid_for_inference': False,
                        'issues': ['invalid-cell-evidence:' + str(error)], 'requests': None, 'resources': None, 'preload': None}
            cells.append(cell)
            print(json.dumps({'record_type': 'native-analysis-progress', 'processed_cells': index + 1, 'total_cells': len(planned)}), flush=True)
    if evidence.errors or evidence.parse_errors: global_issues.append('artifact-integrity-or-decoding-error')
    if any(row['execution_status'] == 'admission-failed' and not row.get('valid_for_admission_inference') for row in cells):
        global_issues.append('unsupported-admission-continuation')
    for cell in cells:
        if any(row.get('record_type') == 'dataset-not-capacity-eligible' and row.get('dataset') == cell['dataset'] and row.get('model') in (None, cell['model']) for row in events):
            cell['valid_for_inference'] = False
            cell['valid_for_admission_inference'] = False
            if cell['local_guard'] != 'unmeasured': cell['local_guard'] = 'inconclusive'
            cell['issues'] = sorted(set(cell['issues'] + ['canonical-dataset-exclusion']))
        if global_issues:
            cell['valid_for_inference'] = False
            cell['valid_for_admission_inference'] = False
            if cell['local_guard'] != 'unmeasured': cell['local_guard'] = 'inconclusive'
            cell['issues'] = sorted(set(cell['issues'] + global_issues))
    margin = campaign.get('statistical_plan', {}).get('practical_equivalence_margin_ratio')
    if margin is not None and (not OLD.numeric(margin) or margin <= 1): raise ValueError('invalid prospectively frozen practical-equivalence margin')
    # The established helper requires a numeric comparison width. A width of1
    # is used only to obtain its descriptive paired intervals when no margin
    # exists; its equivalence classification is then explicitly suppressed.
    pairs = OLD.pair_tables([dict(row, requests=row['requests'] or {}) for row in cells], margin if margin is not None else 1.0)
    for row in pairs:
        if margin is None: row['latency_equivalence'] = 'not-assessed-no-frozen-margin'
        row['practical_equivalence_margin_ratio'] = margin
        row['scope'] = 'Paired native cells on the declared physical host, fixed server/client CPU pools and identical neutral intended schedules. Equal finite offered rates are not a capacity-equivalence proof.'
    for row in inventories.values(): row['valid_for_storage_inference'] = bool(row['storage_inventory_consistent'] and not global_issues)
    authorization = []
    for dataset in campaign['corpora']:
        rows = {model: inventories[dataset['id'], model] for model in ('wac', 'acp')}
        authorization.append({'dataset': dataset['id'], 'both_sampled_neutral_oracles_passed': all(row['complete'] for row in rows.values()),
            'checks_by_model': {model: row['verification']['checks'] for model, row in rows.items()},
            'scope': 'Exact bags or ordered rows and independent counts on the declared intensity-extrema samples under common intended rights. This is separate from performance similarity and does not establish every possible WAC/ACP policy equivalence.'})
    campaign.pop('_sha256'); campaign.pop('_source_bindings')
    return {'schema_version': 1, 'analysis_kind': 'native-preloaded-campaign-independent-accounting',
        'campaign_id': campaign['campaign_id'], 'campaign_sha256': campaign_hash, 'campaign': campaign,
        'source_commit': campaign['source_commit'], 'binary_build_source_commit': campaign['binary_build_source_commit'], 'binary_sha256': campaign['binary_sha256'],
        'source_input_verification': binding,
        'source_review': {'status': review_status, 'matches_exact_result': review_matches, 'record': review,
                          'review_sha256': OLD.sha_file(Path(review_path)) if review_path else None, 'quarantine_events': quarantines},
        'artifact_integrity': {'complete': evidence.manifest_status == 'verified' and not evidence.errors and not evidence.parse_errors,
            'manifest_status': evidence.manifest_status, 'errors': evidence.errors, 'parsing_errors': evidence.parse_errors,
            'parsed_input_sha256': dict(sorted(evidence.hashes.items()))},
        'execution_status': completion.get('status'), 'global_issues': sorted(set(global_issues)), 'events': events,
        'analysis_sources_sha256': {str(path.relative_to(ROOT)): OLD.sha_file(path) for path in (Path(__file__), Path(__file__).with_name('run-campaign.py'), Path(__file__).with_name('run-cell.py'), ROOT / 'bench/ac/million/campaign_analysis.py')},
        'corpora': list(inventories.values()), 'sampled_authorization_correctness': authorization, 'cells': cells, 'paired_comparisons': pairs,
        'headline_eligible_cells': [row['label'] for row in cells if row['valid_for_inference']],
        'admission_failure_cells': [row['label'] for row in cells if row.get('valid_for_admission_inference')],
        'retained_history_million_pod_admission': retained_history_admission(campaign, cells, binding),
        'scope': 'Complete execution alone admits no capacity claim. Only fully checked, reviewed native cells can support scoped inferences; partial/rejected evidence remains descriptive. Network journeys and general all-service representativeness remain separate questions.'}
