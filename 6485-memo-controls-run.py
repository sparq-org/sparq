import datetime
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import time

P = Path(__file__).resolve().parent
R = P.parents[1]
W = R / 'worktrees/issue6485'
FLOOR = 2147483648
START_FLOOR = 2214592512
MAX_BYTES = 268435456

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def write(name, value):
    (P / name).write_text(json.dumps(value, indent=2) + '\n')

def resources():
    return {'free_bytes': shutil.disk_usage(P).free,
            'allocated_bytes': sum(p.stat().st_blocks * 512 for p in P.rglob('*') if p.is_file())}

def stamp():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()

def main():
    initial = resources()
    assert initial['free_bytes'] >= START_FLOOR, initial
    assert initial['allocated_bytes'] <= MAX_BYTES, initial
    source = json.loads((P / 'source-proof.json').read_text())
    compiler = json.loads((P / 'compiler-proof.json').read_text())
    libs = json.loads((P / 'reused-artifacts.json').read_text())
    for f in source['files']:
        assert sha(P / 'core' / f['path']) == f['sha256'], f['path']
        assert sha(W / 'crates/sparq-core' / f['path']) == f['base_sha256'], f['path']
    assert sha(P / 'core/README.md') == source['readme_sha256']
    assert sha(compiler['compiler']) == compiler['sha256']
    for lib in libs:
        assert sha(lib['path']) == lib['sha256'], lib['crate']
    head = subprocess.check_output(['git', '-C', str(W), 'rev-parse', 'HEAD'], text=True).strip()
    status = subprocess.check_output(['git', '-C', str(W), 'status', '--porcelain'], text=True)
    assert head == source['head'] and not status, (head, status)
    recorded_env = json.loads((P / 'recorded-environment.json').read_text())
    recorded_env.update(CARGO_MANIFEST_DIR=str(W / 'crates/sparq-core'),
                        CARGO_MANIFEST_PATH=str(W / 'crates/sparq-core/Cargo.toml'),
                        CARGO_NET_OFFLINE='true', CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='1',
                        RAYON_NUM_THREADS='1', TMPDIR=str(P / 'tmp'), RUST_BACKTRACE='0')
    env = os.environ.copy()
    for key in list(env):
        if key.startswith('SPARQ_') or key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUST_TEST_THREADS'):
            del env[key]
    env.update(recorded_env)
    write('preexecution-proof.json', {'at': stamp(), 'resources': initial,
          'head': head, 'clean': not status, 'compiler_and_7_externs_rehashed': True,
          'all_15_core_sources_rehashed': True, 'selected_environment': recorded_env})
    started = time.monotonic()
    deadline = started + 480
    receipts = []
    samples = []
    def run(name, argv):
        before = resources()
        assert before['free_bytes'] >= FLOOR and before['allocated_bytes'] <= MAX_BYTES
        began = time.monotonic()
        receipt = {'name': name, 'argv': argv, 'cwd': str(W), 'started': stamp(), 'before': before}
        write(name + '-command.json', receipt)
        stop = None
        with (P / (name + '.stdout')).open('w') as out, (P / (name + '.stderr')).open('w') as err:
            proc = subprocess.Popen(argv, cwd=W, env=env, stdout=out, stderr=err, start_new_session=True)
            receipt['pid'] = proc.pid
            last = 0
            while proc.poll() is None:
                now = time.monotonic()
                resource = resources()
                samples.append({'phase': name, 'elapsed_seconds': now-started, **resource})
                if resource['free_bytes'] < FLOOR:
                    stop = 'free-disk-floor'
                elif resource['allocated_bytes'] > MAX_BYTES:
                    stop = 'allocated-output-limit'
                elif now >= deadline:
                    stop = 'aggregate-time-limit'
                if stop:
                    os.killpg(proc.pid, signal.SIGTERM)
                    try:
                        proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(proc.pid, signal.SIGKILL)
                        proc.wait()
                    break
                if now-last >= 30:
                    print(json.dumps({'phase': name, 'elapsed_seconds': now-started, **resource}), flush=True)
                    last = now
                time.sleep(0.5)
            code = proc.wait()
        receipt.update(exit_code=code, stop_reason=stop, ended=stamp(),
                       duration_seconds=time.monotonic()-began, after=resources(), terminal=True)
        receipts.append(receipt)
        write(name + '-receipt.json', receipt)
        write('resources.json', samples)
        print(json.dumps(receipt), flush=True)
        return code == 0 and stop is None
    outcome = 'compile-failed'
    compile_argv = json.loads((P / 'compile-argv.json').read_text())
    if run('compile', compile_argv):
        binary = P / 'artifacts/sparq_core-memo_controls_f50'
        write('binary.json', {'path': str(binary), 'bytes': binary.stat().st_size, 'sha256': sha(binary)})
        for f in source['files']:
            assert sha(P / 'core' / f['path']) == f['sha256']
        ok = run('test', [str(binary), 'has_high_precision_decimal_memo', '--nocapture', '--test-threads=1'])
        outcome = 'test-failed'
        if ok:
            output = (P / 'test.stdout').read_text()
            outcome = 'passed-two-tests' if 'test result: ok. 2 passed; 0 failed;' in output else 'invalid-zero-or-missing-test-count'
    write('phase-result.json', {'outcome': outcome, 'ended': stamp(), 'elapsed_seconds': time.monotonic()-started,
          'receipts': receipts, 'final_resources': resources(), 'commands_pending': False,
          'retries': 0, 'source_modified_after_preregistration': False})
    print(json.dumps({'outcome': outcome, 'commands_pending': False}), flush=True)

try:
    main()
except Exception as exc:
    write('setup-or-export-error.json', {'type': type(exc).__name__, 'error': str(exc), 'at': stamp(), 'resources': resources()})
    raise
