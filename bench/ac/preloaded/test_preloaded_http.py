#!/usr/bin/env python3
"""[GPT-6] Functional HTTP preload/drain/replay checks; never a timing result."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

SPEC = importlib.util.spec_from_file_location('preloaded_cell', Path(__file__).with_name('run-cell.py'))
CELL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CELL)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, default=Path(os.environ.get('CARGO_TARGET_DIR', 'target')) / 'release/examples/pod_population_http')
    parser.add_argument('--workload', type=Path, default=CELL.ROOT / 'bench/ac/million/workload.json')
    args = parser.parse_args()
    binary = str(args.binary.resolve())
    with tempfile.TemporaryDirectory(prefix='sparq-preloaded-http-') as temporary:
        root = Path(temporary)

        def run(command, label):
            completed = subprocess.run([binary, *command], text=True, capture_output=True, timeout=180)
            (root / (label + '.log')).write_text(completed.stdout + completed.stderr)
            if completed.returncode:
                print(completed.stdout, completed.stderr, flush=True)
                raise RuntimeError(f'{label} exited {completed.returncode}')
            rows = []
            for line in completed.stdout.splitlines():
                try: rows.append(json.loads(line))
                except json.JSONDecodeError: pass
            print(json.dumps({'check': label, 'exit_code': 0, 'records': len(rows)}), flush=True)
            return rows

        run(['auth', '--auth-dir', str(root / 'auth')], 'auth')
        secret = root / 'auth/operator-token'
        descriptor = os.open(secret, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, 'w') as stream:
            stream.write(secrets.token_hex(32))
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0)); port = listener.getsockname()[1]
        address = f'127.0.0.1:{port}'; connect = f'http://{address}'

        for model in ('wac', 'acp'):
            corpus = root / model
            run(['pack', '--corpus', str(corpus), '--profile', 'smoke', '--pods', '8', '--model', model], model + '-pack')
            run(['prepare-native', '--corpus', str(corpus)], model + '-prepare')
            run(['verify', '--corpus', str(corpus), '--storage-mode', 'native', '--verify-pods', '8',
                 '--workload-file', str(args.workload)], model + '-native-oracle')
            server = None; server_log = None

            def stop():
                nonlocal server, server_log
                if server is not None:
                    server.terminate()
                    try: server.wait(timeout=10)
                    except subprocess.TimeoutExpired: server.kill(); server.wait(timeout=5)
                    server = None
                if server_log is not None:
                    server_log.close(); server_log = None

            def start(phase):
                nonlocal server, server_log
                path = root / f'{model}-{phase}-server.jsonl'
                server_log = path.open('w')
                server = subprocess.Popen([binary, 'serve', '--corpus', str(corpus), '--auth-dir', str(root / 'auth'),
                    '--storage-mode', 'native', '--workers', '2', '--bind', address,
                    '--control-token-file', str(secret)], stdout=server_log, stderr=subprocess.STDOUT)
                deadline = time.monotonic() + 60
                while time.monotonic() < deadline:
                    if server.poll() is not None:
                        raise RuntimeError(path.read_text())
                    rows = []
                    for line in path.read_text().splitlines():
                        try: rows.append(json.loads(line))
                        except json.JSONDecodeError: pass
                    if any(row.get('record_type') == 'server-ready' for row in rows):
                        ready = [row for row in rows if row.get('record_type') == 'all-population-ready']
                        assert len(ready) == 1 and ready[0]['population'] == ready[0]['retained_pods'] == 8, rows
                        assert len([row for row in rows if row.get('record_type') == 'native-archive-verified']) == 1, rows
                        print(json.dumps({'check': model + '-' + phase, 'ready': ready[0]}), flush=True)
                        return
                    time.sleep(.05)
                raise TimeoutError('all-population readiness absent')

            def drain(phase, previous=None):
                rows = run(['drain', '--connect', connect, '--control-token-file', str(secret),
                            '--drain-timeout-seconds', '30'], model + '-' + phase)
                assert len(rows) == 1, rows
                CELL.validate_barrier(rows[0], 8, 2, 'native', previous)
                print(json.dumps({'check': model + '-' + phase, 'barrier': rows[0]}), flush=True)
                return rows[0]

            def churn(state, identity, delta=None):
                command = ['churn', '--corpus', str(corpus), '--auth-dir', str(root / 'auth'),
                           '--connect', connect, '--state', state, '--mutation-id', identity]
                if delta is not None: command += ['--expected-delta', str(delta)]
                rows = run(command, model + '-' + identity)
                # The CLI checks owner/recipient/anonymous results and rejects
                # recipient, foreign owner and unauthenticated policy mutation.
                assert rows[-1]['record_type'] == 'policy-churn-check-complete', rows
                print(json.dumps({'check': model + '-' + identity, 'probes': rows}), flush=True)

            try:
                start('initial'); before = drain('initial-drain')
                request = urllib.request.Request(connect + '/__benchmark/drain', data=b'', method='POST')
                try: urllib.request.urlopen(request, timeout=2)
                except urllib.error.HTTPError as error: assert error.code == 403, error
                else: raise AssertionError('anonymous operator drain succeeded')
                churn('revoke', 'revoke-before-restart', -1)
                drain('revoked-drain', before)
                stop(); start('replay'); replay = drain('replay-drain')
                assert sum(row['state']['activations']['journal_entries_replayed'] for row in replay['workers']) == 1, replay
                churn('probe-revoked', 'revoked-after-restart')
                churn('grant', 'grant-after-restart', 1)
                drain('restored-drain', replay)
                stop()
                # Independent mutation/audit lane starts from the same archive.
                # Earlier explicit churn receipts are deliberately not mixed into
                # the journey receipt reconciliation universe.
                shutil.rmtree(corpus / 'updates')
                start('journeys'); before = drain('journeys-before')
                raw = root / (model + '-journeys.jsonl')
                run(['load', '--corpus', str(corpus), '--auth-dir', str(root / 'auth'), '--connect', connect,
                     '--mix', 'journeys', '--requests', '512', '--rate', '20', '--seed', '2026090601',
                     '--mutation-epoch', '1', '--workload-file', str(args.workload), '--out', str(raw)], model + '-journeys')
                after = drain('journeys-after', before)
                assert sum(row['processed_requests'] for row in after['workers']) == 512, after
                rows = [json.loads(line) for line in raw.read_text().splitlines()]
                requests = [row for row in rows if row.get('record_type') == 'request']
                assert len(requests) == 512 and all(row.get('status') == 200 for row in requests), requests
                mutations = [row for row in requests if row.get('planned_records')]
                assert mutations and all(row.get('mutation_receipt_present') for row in mutations), mutations
                stop(); start('journey-replay'); replay = drain('journey-replay-drain')
                assert sum(row['state']['activations']['journal_entries_replayed'] for row in replay['workers']) > 0, replay
                stop()
                audit_rows = run(['audit', '--corpus', str(corpus), '--storage-mode', 'native'], model + '-audit')
                audit = root / (model + '-audit.jsonl')
                audit.write_text(''.join(json.dumps(row) + '\n' for row in audit_rows))
                reconciliation = CELL.BASE.reconcile(raw, audit)
                assert reconciliation['passed'], reconciliation
                print(json.dumps({'check': model + '-durable-receipts', 'reconciliation': reconciliation}), flush=True)
            finally:
                stop()
        print(json.dumps({'record_type': 'preloaded-http-smoke-complete', 'models': ['wac', 'acp'],
            'pods_per_model': 8, 'scope': 'functional correctness only; no performance claim'}), flush=True)


if __name__ == '__main__':
    main()
