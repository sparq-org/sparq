from pathlib import Path
import hashlib
import json
import os
import shutil
import signal
import subprocess
import time

ROOT = Path(__file__).resolve().parent
WORKTREE = Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246')
RUSTC = '/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc'
PREVIOUS_SECONDS = json.loads((ROOT / 'preformat/results.json').read_text())['elapsed_seconds']
MIN_FREE = 8 * 1024**3
MAX_BYTES = 128 * 1024**2
out = ROOT / 'final-outputs'
(out / 'tmp').mkdir(parents=True)
env = dict(os.environ, TMPDIR=str(out / 'tmp'), RUSTUP_TOOLCHAIN='1.97.1', RAYON_NUM_THREADS='1')
started = time.monotonic()
minimum_free = shutil.disk_usage(ROOT).free
records = []
stop_reason = None

def evidence_bytes():
    return sum(p.stat().st_size for p in ROOT.rglob('*') if p.is_file())

def limit():
    global minimum_free
    free = shutil.disk_usage(ROOT).free
    minimum_free = min(minimum_free, free)
    if free < MIN_FREE:
        return 'free disk below 8 GiB'
    if evidence_bytes() > MAX_BYTES:
        return 'task output exceeds 128 MiB'
    if time.monotonic() - started + PREVIOUS_SECONDS > 120:
        return 'total verification time exceeds 120 seconds'
    return None

version = subprocess.check_output([RUSTC, '-vV'], text=True)
test_args = ['--exact', 'tests::window_ownership_and_calibration', '--test-threads=1', '--nocapture']
commands = [
    ('compile-fixed', [RUSTC, '--edition=2021', '--test', '-D', 'warnings', str(WORKTREE / 'bench/overlay-count/src/counting.rs'), '-o', str(out / 'fixed-tests')], 0),
    ('run-fixed', [str(out / 'fixed-tests'), *test_args], 0),
    ('compile-old', [RUSTC, '--edition=2021', '--test', '-D', 'warnings', str(ROOT / 'old_counting_with_tests.rs'), '-o', str(out / 'old-tests')], 0),
    ('run-old', [str(out / 'old-tests'), *test_args], 101),
]
for name, command, expected in commands:
    stop_reason = limit()
    if stop_reason:
        break
    before = time.monotonic()
    with (ROOT / ('final-' + name + '.log')).open('w') as log:
        process = subprocess.Popen(command, cwd=WORKTREE, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        while process.poll() is None:
            stop_reason = limit()
            if stop_reason:
                os.killpg(process.pid, signal.SIGKILL)
                break
            time.sleep(0.05)
        code = process.wait()
    record = dict(name=name, command=command, expected_exit=expected, exit=code,
                  seconds=time.monotonic() - before, free_bytes_after=shutil.disk_usage(ROOT).free)
    records.append(record)
    print(json.dumps(record), flush=True)
    if stop_reason or code != expected:
        stop_reason = stop_reason or 'unexpected command exit'
        break
stop_reason = stop_reason or limit()
source = (ROOT / 'fixed_counting.rs').read_bytes()
assert source == (WORKTREE / 'bench/overlay-count/src/counting.rs').read_bytes()
suffix = (ROOT / 'exact_test_suffix.rs').read_bytes()
old = (ROOT / 'old_counting.rs').read_bytes()
assert (ROOT / 'old_counting_with_tests.rs').read_bytes() == old + b'\n' + suffix
assert source.endswith(suffix)
result = dict(base_head='5757e70be09ed95bcbc2831cec7850fa29fdf620', rustc_version=version,
              limits=dict(total_seconds=120, output_bytes=MAX_BYTES, minimum_free_bytes=MIN_FREE),
              previous_verification_seconds=PREVIOUS_SECONDS, elapsed_seconds=time.monotonic() - started,
              combined_verification_seconds=PREVIOUS_SECONDS + time.monotonic() - started,
              minimum_observed_free_bytes=minimum_free, final_free_bytes=shutil.disk_usage(ROOT).free,
              total_evidence_bytes=evidence_bytes(), results=records, stop_reason=stop_reason,
              old_source_prefix_exact=True, identical_actual_test_suffix=True,
              sources_sha256={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in ROOT.glob('*.rs')})
(ROOT / 'final-results.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2), flush=True)
raise SystemExit(0 if not stop_reason and len(records) == 4 else 1)
