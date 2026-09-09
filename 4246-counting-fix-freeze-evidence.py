from pathlib import Path
import hashlib
import json
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent
WT = Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246')
BASE = '5757e70be09ed95bcbc2831cec7850fa29fdf620'

def git(*args):
    return subprocess.check_output(['git', *args], cwd=WT)

def digest(data):
    return hashlib.sha256(data).hexdigest()

head = git('rev-parse', 'HEAD').decode().strip()
assert git('status', '--porcelain') == b''
assert git('diff', '--binary', BASE, head) == (ROOT / 'full.diff').read_bytes()
assert git('show', head + ':bench/overlay-count/src/counting.rs') == (ROOT / 'fixed_counting.rs').read_bytes()
assert git('show', BASE + ':bench/overlay-count/src/counting.rs') == (ROOT / 'old_counting.rs').read_bytes()
assert git('show', head + ':bench/overlay-count/README.md') == (ROOT / 'README.md.source').read_bytes()
(ROOT / 'commit.txt').write_bytes(git('show', '--no-patch', '--format=fuller', head))
(ROOT / 'final-diff-check.log').write_bytes(subprocess.check_output(['git', 'diff', '--check', BASE, head], cwd=WT))
results = json.loads((ROOT / 'final-results.json').read_text())
assert [r['exit'] for r in results['results']] == [0, 0, 0, 101]
assert results['stop_reason'] is None
report = {
    'head': head,
    'base': BASE,
    'provenance': 'Implemented and validated by the actual inherited OpenAI GPT-6 Astra xhigh runtime. No independent reviewer/model call performed in this task.',
    'decision': 'Focused bench-only correction ready for independent review; no new measurement or admission claim.',
    'change': 'A separate WINDOW_OPEN CAS acquires ownership before all counter reset and releases only after all five end counters are read. ACTIVE boundaries and allocator hot path remain unchanged. README documents the direct module regression and contract.',
    'diff': {'files': 2, 'insertions': 126, 'deletions': 4},
    'validation': {
        'final_candidate': '1 passed, 0 failed, 0 filtered; four clean calibrations plus nested and synchronized competing denied admission.',
        'old_source_control': 'Compiled successfully with exact old production and byte-identical final test suffix; 1 failed, 0 passed, 0 filtered. Four clean calibrations passed before behavioral failure.',
        'known_request': 'After each denied admission, fixed ACTIVE=true and allocation/reallocation/requested-byte deltas=(1,0,128); old ACTIVE=false and deltas=(0,0,0). Invalid-window totals are deliberately not treated as exact allocation evidence.',
        'format': 'Pinned rustfmt --check on counting.rs and git diff --check pass. Only touched Rust file formatted.',
        'compiler_warning': 'Candidate compilation exits 0 but emits Xcode FSEvents/DARWIN_USER_CACHE_DIR linker warnings. Rust linker_messages ignores -D warnings; full verbatim logs preserved. No claim of warning-free linker output.',
        'not_run': 'No Cargo, Clippy, full preflight, optimized or wasm build, dependency install, benchmark remeasurement, or remote action.',
        'ci_scope': 'This is a detached bench workspace. Ordinary root workspace tests/lints do not execute this regression. Exact direct rustc command is documented; no workflow wiring changed.',
    },
    'resource_limits': results['limits'],
    'resource_actual': {
        'total_two_round_seconds': results['combined_verification_seconds'],
        'minimum_observed_free_bytes_during_final': results['minimum_observed_free_bytes'],
        'final_test_free_bytes': results['final_free_bytes'],
        'first_round_preserved': 'preformat/ contains unchanged first source, binary and log bytes. A second pair binds results to the real formatting change, not a retry of failed/noisy tests.',
    },
    'caller_assessment': [
        'main.rs initializes one Rayon worker and calibrates before dispatch. Its normal sample loop synchronously calls begin/run/end.',
        'lifecycle.rs measure() is the only lifecycle begin/end wrapper. run_all and reads-per-generation invoke it serially.',
        'The concurrent-cold workload spawns two readers inside one measured closure and joins both before returning; neither reader admits or ends a window.',
        'calibrate() uses a balanced begin/end and is byte-identical to base.',
    ],
    'historical_measurements': 'No bug-triggering overlapping or nested coordinator call was found in existing production callers. The old implementation also passes the four clean calibrations here. This supports retaining existing exact-old-head single-coordinator measurements with their original global-allocator/quiescence limits; it is not a historical execution trace, proof against incidental process allocations, or corrected-head remeasurement. All frozen prior data remain unchanged.',
    'limits': [
        'Balanced end by the successful coordinator with workers quiescent is still required; no owner token, RAII or non-owner end enforcement is added.',
        'The ownership guard prevents competing begin from resetting counters during initialization/readout. ACTIVE and counter operations do not provide an atomic snapshot under arbitrary concurrent allocations.',
        'The competing regression is synchronized after owner begin returns. It does not force reset/readout interleavings, nor use timing thresholds or probabilistic stress.',
        'Panic hooks/payloads and thread machinery can allocate. Known-request probes run after that machinery; clean calibration, not panic-window totals, is the exact oracle.',
        'Libtest is process-global; serial one-test invocation and successful calibration are evidence for this execution, not a guarantee against all harness noise.',
    ],
    'prior_assessment': {
        'path': '../counting-review/review-packet.md',
        'sha256': '0fe3faa01107827ef0c90168965030ef806683d021d11f5dda0a10689d31175d',
    },
    'clean_worktree': True,
}
(ROOT / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
(ROOT / 'report.md').write_text(
    f'Local head `{head}` fixes allocation-window ownership in two bench-only files. '
    'The allocator hot path, workload callers, core source and feature-off declaration are unchanged.\n\n'
    'Exact formatted source: one serial regression passes. Unchanged old production with the identical test body compiles and fails the post-denial allocation probes. '
    'Both pass four clean calibrations. Fixed probes record one 128-byte request after each denied begin; old probes record none.\n\n'
    f'The original and formatted verification pairs used {results["combined_verification_seconds"]:.3f} seconds total. '
    'All task-private output stays below 128 MiB and observed free disk above 8 GiB. Pinned targeted rustfmt and whitespace checks pass. '
    'Xcode linker warnings are preserved; compiler success is not a Clippy result. Ordinary workspace CI does not reach this detached regression.\n\n'
    'Existing measurements remain exact-old-head evidence: current callers have one balanced coordinator and joined workers; no trigger for this bug was found there. '
    'No benchmark was rerun. Ownership/quiescence preconditions remain, and the test does not force every initialization/readout interleaving.\n'
)
sections = [
    ('Review brief', 'report.md', 'text'),
    ('Detailed findings and limits', 'report.json', 'json'),
    ('Commit', 'commit.txt', 'text'),
    ('Complete two-file diff', 'full.diff', 'diff'),
    ('Complete fixed allocator module and regression', 'fixed_counting.rs', 'rust'),
    ('Complete original allocator', 'old_counting.rs', 'rust'),
    ('Exact compiled old-source control delta', 'old-source-control.diff', 'diff'),
    ('Full main caller', 'context/bench/overlay-count/src/main.rs', 'rust'),
    ('Full lifecycle callers', 'context/bench/overlay-count/src/lifecycle.rs', 'rust'),
    ('Detached manifest', 'context/bench/overlay-count/Cargo.toml', 'toml'),
    ('All local begin/end/caller references', 'caller-search.txt', 'text'),
    ('Scope verification', 'scope-verification.json', 'json'),
    ('Final exact commands, toolchain and resource results', 'final-results.json', 'json'),
    ('Final candidate compile output', 'final-compile-fixed.log', 'text'),
    ('Final candidate test output', 'final-run-fixed.log', 'text'),
    ('Final old-source compile output', 'final-compile-old.log', 'text'),
    ('Final old-source behavioral failure', 'final-run-old.log', 'text'),
    ('Formatting commands', 'format-checks.json', 'json'),
    ('Actual lint scope and exclusions', 'lint-scope.json', 'json'),
    ('CI lint/format source excerpt', 'ci-lint-scope.txt', 'text'),
    ('Root workspace source excerpt', 'workspace-scope.txt', 'text'),
    ('Markdown source scope', 'markdown-scope.txt', 'text'),
]
packet = '# PR6469 allocator ownership correction — focused review\n\n'
packet += f'Frozen source `{head}`, delta from `{BASE}`. No new performance claim.\n\n'
for title, filename, language in sections:
    packet += f'## {title}\n\nFile: `{filename}`\n\n```{language}\n' + (ROOT / filename).read_text() + '\n```\n\n'
(ROOT / 'review-packet.md').write_text(packet)
entries = []
for p in sorted(ROOT.rglob('*')):
    if p.is_file() and p.name not in ('manifest.json', 'freeze.json'):
        entries.append({'path': str(p.relative_to(ROOT)), 'bytes': p.stat().st_size, 'sha256': digest(p.read_bytes())})
(ROOT / 'manifest.json').write_text(json.dumps({'head': head, 'files': entries}, indent=2) + '\n')
total = sum(x['bytes'] for x in entries)
assert total < 128 * 1024**2
free = shutil.disk_usage(ROOT).free
assert free >= 8 * 1024**3
freeze = {'head': head, 'base': BASE, 'packet_path': str(ROOT / 'review-packet.md'),
          'packet_bytes': (ROOT / 'review-packet.md').stat().st_size,
          'packet_sha256': digest((ROOT / 'review-packet.md').read_bytes()),
          'manifest_sha256': digest((ROOT / 'manifest.json').read_bytes()),
          'manifest_entries': len(entries), 'total_manifest_bytes': total,
          'free_bytes_at_freeze': free, 'clean_worktree': True}
(ROOT / 'freeze.json').write_text(json.dumps(freeze, indent=2) + '\n')
print(json.dumps(freeze, indent=2))
