#!/usr/bin/env python3
"""[GPT-6] Run one preloaded cell using the frozen journey/receipt machinery."""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import secrets
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('population_campaign', ROOT / 'bench/ac/million/run-campaign.py')
BASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BASE)


def selected_cell(args, spec):
    """Reject wrong identities/resources before creating output or resetting journals."""
    preloaded = spec['preloaded']; host = spec['host']
    if preloaded['version'] != 1 or not spec['campaign_id'].startswith('solid-pod-preloaded-'):
        raise ValueError('a separately named, versioned preloaded campaign is required')
    servers = host['server_cpu_pool']; clients = host['client_cpu_pool']
    cpus = preloaded['fixed_server_cpus']
    if cpus != 16 or len(servers) != cpus or len(clients) != 4 or len(set(servers + clients)) != cpus + 4:
        raise ValueError('require fixed 16 server and 4 distinct client CPUs')
    if any(type(cpu) is not int or cpu < 0 or cpu >= host['physical_vcpus'] for cpu in servers + clients):
        raise ValueError('CPU outside declared host')
    fraction = host['overall_memory_fraction']
    if host['swap'] is not False or not 0 < fraction < 1 or args.memory_gib >= host['physical_memory_gib'] * fraction:
        raise ValueError('server tier must leave space within the outer RAM bound; swap must be disabled')
    datasets = {d['id']: d for d in spec['corpora']}; groups = {g['id']: g for g in spec['groups']}
    if len(datasets) != len(spec['corpora']) or len(groups) != len(spec['groups']):
        raise ValueError('duplicate dataset or group identity')
    dataset = datasets[args.dataset]; group = groups[args.group]
    if args.model not in dataset['models'] or args.dataset not in group['datasets'] or args.memory_gib not in group['memory_gib'] or args.memory_gib <= 0:
        raise ValueError('requested cell is outside the declared campaign')
    if args.replicate not in range(group['repeat']) or args.replicate >= len(spec['seeds']) or group['cpus'] != [cpus]:
        raise ValueError('requested repetition or CPU allocation is outside the declared campaign')
    if group['storage_mode'] not in ('native', 'memory'):
        raise ValueError('preloaded group requires native or memory storage')
    rate = 'derived' if args.rate == 'derived' else float(args.rate)
    if rate not in group.get('rates', ['derived']):
        raise ValueError('requested rate is outside the declared campaign')
    label = f'{dataset["id"]}-{group["id"]}-ram{args.memory_gib}-cpu{cpus}-r{rate}-{args.replicate}-{args.model}'
    if any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._' for c in label):
        raise ValueError('cell label contains path syntax')
    if args.results.exists() and any(args.results.iterdir()):
        raise ValueError('one-cell output directory must be empty; never overwrite prior evidence')
    return dataset, group, rate, label


def validate_barrier(record, population, workers, mode, previous=None):
    """Require complete retained ownership and no timed activation at a drain."""
    if record.get('record_type') != 'worker-drain-complete' or record.get('passed') is not True:
        raise ValueError('drain did not complete successfully')
    rows = record.get('workers', [])
    if len(rows) != workers:
        raise ValueError('missing worker drain acknowledgement')
    retained = 0
    for index, row in enumerate(rows):
        state = row['state']; counts = state['activations']
        expected = len(range(index, population, workers))
        if state['storage_mode'] != mode or state['population'] != population or state['ready'] is not True or state['poisoned'] is not False:
            raise ValueError('worker is not ready for this exact population/mode')
        if state['retained_pods'] != expected or counts['initial_authorizations'] != expected:
            raise ValueError('worker omitted or duplicated population ownership/authorization')
        if counts['attempted_activations_after_ready'] != 0:
            raise ValueError('a request attempted lazy activation after ready')
        if counts['rdf_parses'] != (expected if mode == 'memory' else 0) or counts['native_dataset_loads'] != (expected if mode == 'native' else 0):
            raise ValueError('startup did not build exactly the declared data representation')
        if previous is not None:
            old = previous['workers'][index]['state']
            for key in ('rdf_parses', 'native_dataset_loads', 'initial_authorizations', 'journal_entries_replayed'):
                if counts[key] != old['activations'][key]:
                    raise ValueError('data activation or journal replay occurred during measurement')
            if row['processed_requests'] < previous['workers'][index]['processed_requests']:
                raise ValueError('worker processing counter regressed')
        retained += state['retained_pods']
    if retained != population:
        raise ValueError('drain does not retain the entire population')


class PreloadedCell(BASE.Campaign):
    def __init__(self, args):
        spec = json.loads(args.campaign.read_text())
        self.selection = selected_cell(args, spec)
        revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        if revision != args.source_commit:
            raise ValueError('source checkout differs from explicitly selected merged revision')
        super().__init__(args)
        self.preloaded = self.spec['preloaded']
        if self.preloaded['version'] != 1:
            raise ValueError('unknown preloaded campaign version')
        self.control_file = self.auth / 'preloaded-control-token'
        if not self.control_file.exists():
            fd = os.open(self.control_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, 'w') as stream:
                stream.write(secrets.token_hex(32))
        self.before = None
        self.population = None
        self.mode = None
        self.workers = None
        BASE.write_json(self.results / 'preloaded-runtime.json', {
            'version': 1, 'source_commit': revision, 'binary_sha256': BASE.sha(self.binary),
            'campaign_sha256': BASE.sha(args.campaign),
            'runner_sha256': BASE.sha(Path(__file__)),
            'base_runner_sha256': BASE.sha(ROOT / 'bench/ac/million/run-campaign.py'),
            'workload_sha256': BASE.sha(ROOT / 'bench/ac/million/workload.json'),
            'scope': 'all data descriptors and real authorization retained; native content pages may fault; heap metadata does not spill',
        })

    def start_server(self, corpus, group, memory, cpus, label):
        mode = group['storage_mode']
        if mode not in ('native', 'memory') or cpus != self.preloaded['fixed_server_cpus']:
            raise ValueError('storage mode or fixed server CPU count differs from protocol')
        self.mode = mode; self.workers = cpus; self.before = None
        self.population = json.loads((corpus / 'manifest.json').read_text())['pods']
        if mode == 'native' and not (corpus / 'pods.native-manifest.json').exists():
            raise ValueError('native preparation must finish before a timed cell is launched')
        self.counter += 1
        unit = f'sparq-pod-preloaded-{os.getpid()}-{self.counter}.service'
        self.current_server = unit
        log = self.results / f'{label}-server.log'
        if log.exists():
            raise ValueError('cell server log already exists; readiness cannot be reused')
        command = ['sudo', 'systemd-run', '--quiet', f'--unit={unit}', '--slice=sparq-pod-bench.slice',
            f'--uid={os.getuid()}', f'--gid={os.getgid()}', f'--working-directory={ROOT}',
            '--property=MemoryAccounting=yes', '--property=CPUAccounting=yes', '--property=IOAccounting=yes',
            f'--property=MemoryMax={memory * 1024**3}', '--property=MemorySwapMax=0',
            f'--property=StandardOutput=append:{log}', f'--property=StandardError=append:{log}',
            'taskset', '-c', ','.join(map(str, self.spec['host']['server_cpu_pool'][:cpus])),
            str(self.binary), 'serve', '--corpus', str(corpus), '--auth-dir', str(self.auth),
            '--workers', str(cpus), '--storage-mode', mode, '--control-token-file', str(self.control_file),
            '--max-pod-bytes', str(group['max_pod_bytes']),
            '--queue-capacity', str(self.measurement['queue_capacity_per_worker'])]
        subprocess.run(command, check=True)
        deadline = time.monotonic() + self.preloaded['startup_timeout_seconds']
        while time.monotonic() < deadline:
            self.guard()
            records = []
            if log.exists():
                for line in log.read_text().splitlines():
                    try: records.append(json.loads(line))
                    except json.JSONDecodeError: pass
            ready = [r for r in records if r.get('record_type') == 'all-population-ready']
            listening = any(r.get('record_type') == 'server-ready' for r in records)
            if ready and listening:
                if len(ready) != 1 or ready[0]['population'] != self.population or ready[0]['retained_pods'] != self.population:
                    raise ValueError('server readiness does not cover the complete declared population')
                BASE.write_json(self.results / f'{label}-preload-ready.json', ready[0])
                self.resource(unit, label + '-preload')
                return unit
            active = subprocess.run(['sudo', 'systemctl', 'is-active', unit], capture_output=True, text=True).stdout.strip()
            if active in ('failed', 'inactive'):
                super().resource(unit, label + '-preload-failed')
                raise RuntimeError('preload process exited before complete readiness')
            time.sleep(.5)
        super().resource(unit, label + '-preload-timeout')
        raise TimeoutError('complete population preload exceeded its declared deadline')

    def resource(self, unit, label):
        if label.endswith(('-before', '-after')):
            output = self.results / f'{label}-drain.jsonl'
            status = self.run([str(self.binary), 'drain', '--control-token-file', str(self.control_file),
                '--drain-timeout-seconds', str(self.preloaded['drain_timeout_seconds'])],
                output, timeout=self.preloaded['drain_timeout_seconds'] + 10)
            if status:
                raise RuntimeError('complete worker drain failed; measurement not admitted')
            record = json.loads(output.read_text())
            validate_barrier(record, self.population, self.workers, self.mode,
                             self.before if label.endswith('-after') else None)
            if label.endswith('-before'):
                self.before = record
        return super().resource(unit, label)


def main():
    parser = argparse.ArgumentParser()
    for name in ('campaign', 'results', 'corpora', 'auth', 'binary'):
        parser.add_argument('--' + name, type=Path, required=True)
    for name in ('dataset', 'group', 'model', 'source-commit'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--memory-gib', type=int, required=True)
    parser.add_argument('--replicate', type=int, required=True)
    parser.add_argument('--rate', default='derived')
    args = parser.parse_args()
    campaign = PreloadedCell(args)
    dataset, group, rate, _label = campaign.selection
    corpus = args.corpora / f'{args.dataset}-{args.model}'
    manifest = json.loads((corpus / 'manifest.json').read_text())
    if manifest['pods'] != dataset['pods'] or manifest['model'] != args.model:
        raise ValueError('persisted population/model differs from requested cell')
    # The inherited cell resets task-owned journals. Hold this advisory lock for
    # the entire cell; manually launched servers must be stopped by the operator.
    lock = (corpus / 'preloaded-cell.lock').open('a')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    try:
        outer = int(campaign.spec['host']['physical_memory_gib'] * 1024**3 * campaign.spec['host']['overall_memory_fraction'])
        subprocess.run(['sudo', 'systemctl', 'set-property', '--runtime', 'sparq-pod-bench.slice',
                        f'MemoryMax={outer}', 'MemorySwapMax=0'], check=True)
        result = campaign.cell(dataset, corpus, args.model, group, args.memory_gib,
                               campaign.preloaded['fixed_server_cpus'], rate, args.replicate)
        print(json.dumps(result))
    except BaseException as error:
        campaign.event({'record_type': 'preloaded-cell-stopped', 'dataset': args.dataset,
                        'model': args.model, 'reason': str(error),
                        'scope': 'failed startup/drain is not an observed query percentile or an omitted Pod'})
        raise
    finally:
        campaign.stop_server()
        lock.close()


if __name__ == '__main__':
    main()
