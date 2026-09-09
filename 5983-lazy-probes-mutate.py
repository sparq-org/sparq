"""[GPT-6 Astra] Bounded real source mutants; restore bytes before returning."""
from pathlib import Path
import hashlib
import json
import os
import subprocess
import time

out = Path(__file__).parent
wt = Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/pr5983-lazy-probes')
source = wt / 'crates/sparq-engine/src/exec.rs'
original = source.read_text()
env = dict(os.environ, RAYON_NUM_THREADS='2', CARGO_BUILD_JOBS='2', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0', CARGO_TARGET_DIR=str(out.parent/'implementation/target'))
common = ['/Users/jesght/.cargo/bin/cargo', 'test', '--locked', '--offline', '-p', 'sparq-engine']
records = []

def run(name, args, expected_failure=False):
    command = common + args + ['--', '--test-threads=1']
    started = time.time()
    with (out / (name+'.txt')).open('w') as log:
        process = subprocess.Popen(command, cwd=wt, env=env, stdout=log, stderr=subprocess.STDOUT)
        while process.poll() is None:
            try:
                process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                print(f'{name}: build/test running ({int(time.time()-started)}s)', flush=True)
    text = (out/(name+'.txt')).read_text()
    record = dict(name=name, command=command, exit_code=process.returncode, seconds=time.time()-started, source_sha256=hashlib.sha256(source.read_bytes()).hexdigest())
    records.append(record)
    (out/'mutation-results.json').write_text(json.dumps(records,indent=2)+'\n')
    assert (process.returncode == 101 and 'test result: FAILED.' in text) if expected_failure else process.returncode == 0, text[-4000:]

anchor = '    // The output column list is FIXED across every candidate'
eager = '''    // [GPT-6 Astra] Test counterfactual: force the previous eager placement.
    for op in &mut other_pats {
        if !prepare_other(graph, op, rows.len()) { return Ok(None); }
    }

'''
mutants = [
    ('mutant-eager', anchor, eager+anchor, ['--lib','indexed_topk_preparation_tests']),
    ('mutant-late-cardinality', 'if sub_scan.rows.len().saturating_mul(2) < seed_card {', 'if false && sub_scan.rows.len().saturating_mul(2) < seed_card {', ['--lib','indexed_topk_preparation_tests::late_selective_probe_still_declines_before_emitting']),
    ('mutant-unvisited', 'if other_pats.iter().any(|op| op.prepared.is_none()) {', 'if false && other_pats.iter().any(|op| op.prepared.is_none()) {', ['--test','topk_orderby_indexed_differential','unvisited_lazy_probe_keeps_empty_result_on_fallback']),
]
try:
    run('preparation-final', ['--lib','indexed_topk_preparation_tests'])
    for name, old, new, args in mutants:
        assert original.count(old) == 1
        source.write_text(original.replace(old,new,1))
        run(name,args,True)
        source.write_text(original)
    run('restored-preparation', ['--lib','indexed_topk_preparation_tests'])
    run('compact-preparation', ['--features','sparq-core/compact-index','--lib','indexed_topk_preparation_tests'])
    run('compact-semantic', ['--features','sparq-core/compact-index','--test','topk_orderby_indexed_differential'])
finally:
    source.write_text(original)
    assert source.read_text() == original
    print('Source restored',flush=True)
