from pathlib import Path
import hashlib
import json
import os
import signal
import subprocess
import time

ROOT = Path('/private/tmp/sparq-pr6049/.throughput-monitor')
OUT = ROOT / 'direct-5183/engine-witness'
WORK = ROOT / 'worktrees/issue5183'
DEPS = ROOT / 'direct-5983/implementation/target/release/deps'
RUSTC = Path('/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc')
LIMIT = 64 * 1024 * 1024


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def allocated():
    return sum(p.stat().st_blocks * 512 for p in OUT.rglob('*') if p.is_file())


def git(*args):
    return subprocess.check_output(['git', *args], cwd=WORK, text=True)


assert git('rev-parse', 'HEAD').strip() == '781f667c19a8ebb779cfccb24b05ea432360b025'
assert git('status', '--porcelain') == ''
assert git('diff', '--stat', 'ed66ef0931fa19dd521fac433870c86a78687a30',
           'd41ec9fcb796504d85f5a5247a5fdc9eb3e65de5', '--',
           'crates/sparq-engine', 'crates/sparq-core', 'Cargo.lock') == ''
proof = json.loads((ROOT / 'direct-5183/readiness/source-proof.json').read_text())
assert git('diff', 'd07ca79f89e3a8945be46b516c3cd2f770ccd618',
           '781f667c19a8ebb779cfccb24b05ea432360b025', '--',
           'crates/sparq-engine') == proof['d07_main_engine_diff']
expected = {
    'libsparq_engine-17d69fdf30aada97.rlib': '2135748c75d5afd1ca3731e6d350c5a06b99caf9376b1388fbd472b08febed9d',
    'libsparq_engine-d082493c198a3f5b.rlib': '2f17e066de1c77dd975ec2367c6e140175a31710685b05d8aadccd04f3b7b395',
    'libsparq_core-e9ed6f0c10ca33fd.rlib': '9c6042a15b38ebfddaaef9b24e53df598201b72ec631310f201378c0b06ff7c2',
    'libsparq_core-127354fe878a2f23.rmeta': '1eaf383b2dc290c837c9b923c5ead8fc2ed6a77b677c8dccc847bbbd697e5c8c',
}
for filename, value in expected.items():
    assert digest(DEPS / filename) == value, filename
hashes = {p.name: digest(p) for p in DEPS.glob('*.rlib')}
(OUT / 'prelink-provenance.json').write_text(json.dumps({
    'source_head': git('rev-parse', 'HEAD').strip(),
    'source_status': git('status', '--porcelain'),
    'expected_hashes_verified': expected,
    'all_dependency_rlib_hashes': hashes,
    'main_core_rlib_hash_first_recorded_in_readiness_phase': hashes['libsparq_core-127354fe878a2f23.rlib'],
    'main_core_prior_rmeta_hash_verified': True,
    'rustc_binary_sha256': digest(RUSTC),
    'harness_sha256': digest(OUT / 'witness.rs'),
    'source_proof_sha256': digest(ROOT / 'direct-5183/readiness/source-proof.json'),
    'free_bytes': os.statvfs(OUT).f_bavail * os.statvfs(OUT).f_frsize,
    'allocated_bytes': allocated(),
}, indent=2) + '\n')
start = time.monotonic()
deadline = start + 120
records = []
maximum = allocated()


def run(name, args):
    global maximum
    assert allocated() < LIMIT
    begun = time.monotonic()
    assert begun < deadline
    log = OUT / (name + '.log')
    reason = None
    with log.open('w') as stream:
        process = subprocess.Popen(args, cwd=WORK, stdout=stream, stderr=subprocess.STDOUT,
                                   start_new_session=True,
                                   env={**os.environ, 'RAYON_NUM_THREADS': '1',
                                        'TMPDIR': str(OUT / 'tmp')})
        while process.poll() is None:
            maximum = max(maximum, allocated())
            if time.monotonic() >= deadline or maximum >= LIMIT:
                reason = 'total-time-or-output-limit'
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=5)
                break
            time.sleep(0.1)
        code = process.wait()
    records.append({'name': name, 'argv': [str(a) for a in args],
                    'exit_code': code, 'stop_reason': reason,
                    'elapsed_seconds': time.monotonic() - begun,
                    'log_sha256': digest(log)})
    (OUT / 'commands.json').write_text(json.dumps(records, indent=2) + '\n')
    print(name, code, flush=True)
    assert code == 0 and reason is None, name


(OUT / 'tmp').mkdir()
try:
    run('toolchain', [RUSTC, '-vV'])
    assert 'rustc 1.97.1 (8bab26f4f 2026-07-14)' in (OUT / 'toolchain.log').read_text()
    for variant, engine, core in [
        ('parent', '17d69fdf30aada97', 'e9ed6f0c10ca33fd'),
        ('main', 'd082493c198a3f5b', '127354fe878a2f23'),
    ]:
        binary = OUT / ('witness-' + variant)
        args = [RUSTC, '--edition=2021', '--crate-name', 'update_witness_' + variant,
                OUT / 'witness.rs', '-C', 'opt-level=3', '-C', 'lto=false',
                '-C', 'codegen-units=16', '-C', 'panic=unwind', '-C', 'strip=debuginfo',
                '-L', 'dependency=' + str(DEPS), '--extern',
                'sparq_core=' + str(DEPS / ('libsparq_core-' + core + '.rlib')),
                '--extern', 'sparq_engine=' + str(DEPS / ('libsparq_engine-' + engine + '.rlib')),
                '-o', binary]
        run('link-' + variant, args)
        run('run-' + variant, [binary])
finally:
    (OUT / 'resource-receipt.json').write_text(json.dumps({
        'elapsed_seconds': time.monotonic() - start,
        'limit_seconds': 120,
        'output_limit_bytes': LIMIT,
        'maximum_observed_allocated_bytes': max(maximum, allocated()),
        'free_bytes': os.statvfs(OUT).f_bavail * os.statvfs(OUT).f_frsize,
        'one_process_at_a_time': True,
        'cargo_or_dependency_compilation': False,
    }, indent=2) + '\n')
