"""[GPT-6 Astra] One sequential optimized build lane, total fifteen-minute cap."""
from pathlib import Path
import difflib
import hashlib
import json
import os
import shutil
import subprocess
import time

out = Path(__file__).parent
root = Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees')
new = root/'pr5983-lazy-probes'
source = new/'crates/sparq-engine/src/exec.rs'
original = source.read_text()
head = subprocess.check_output(['git','rev-parse','HEAD'],cwd=new,text=True).strip()
assert not subprocess.check_output(['git','status','--porcelain'],cwd=new,text=True)
harness = ['Cargo.toml','Cargo.lock','README.md','src/main.rs','src/counting.rs']
trees = {'new':new, **{v:root/f'pr5983-lazy-control-{v}' for v in ['dc','disabled','main']}}
hashes = {}
for variant,wt in trees.items():
    for name in harness:
        src = new/'bench/indexed-topk'/name
        dst = wt/'target/bench-wrapper'/name
        dst.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(src,dst)
        digest = hashlib.sha256(dst.read_bytes()).hexdigest()
        assert digest == hashlib.sha256(src.read_bytes()).hexdigest()
        hashes[name] = digest
(out/'harness-hashes.json').write_text(json.dumps(hashes,indent=2)+'\n')
(out/'bins').mkdir(exist_ok=True)
target = out.parent/'diagnostic/target'
env = dict(os.environ,CARGO_BUILD_JOBS='2',RAYON_NUM_THREADS='1',CARGO_TARGET_DIR=str(target))
started = time.time()
records = []
eager_anchor = '    // The output column list is FIXED across every candidate'
eager = '''    // [GPT-6 Astra] Measurement counterfactual: previous eager placement.
    for op in &mut other_pats {
        if !prepare_other(graph, op, rows.len()) { return Ok(None); }
    }

'''
mutated = original.replace(eager_anchor,eager+eager_anchor,1)
assert mutated != original
(out/'eager-control.diff').write_text(''.join(difflib.unified_diff(original.splitlines(True),mutated.splitlines(True),fromfile='new/exec.rs',tofile='eager/exec.rs')))

try:
    for variant,mode in [('new','timing'),('new','count'),('eager','count'),('dc','timing'),('dc','count'),('disabled','timing'),('disabled','count'),('main','timing'),('main','count')]:
        wt = new if variant == 'eager' else trees[variant]
        source.write_text(mutated if variant == 'eager' else original)
        command = ['/Users/jesght/.cargo/bin/cargo','build','--release','--locked','--offline','--manifest-path','target/bench-wrapper/Cargo.toml']
        if mode == 'count': command += ['--features','count-alloc']
        before = time.time()
        with (out/f'build-{variant}-{mode}.txt').open('w') as log:
            proc = subprocess.Popen(command,cwd=wt,env=env,stdout=log,stderr=subprocess.STDOUT)
            while proc.poll() is None:
                if time.time()-started >= 900:
                    proc.terminate()
                    try: proc.wait(timeout=10)
                    except subprocess.TimeoutExpired: proc.kill(); proc.wait()
                    raise SystemExit('Stop: optimized build reached fifteen-minute total cap')
                try: proc.wait(timeout=20)
                except subprocess.TimeoutExpired: print(f'{variant}-{mode}: building; elapsed total={int(time.time()-started)}s',flush=True)
        assert proc.returncode == 0, f'Build failed: {variant}-{mode}; no automatic retry'
        binary = out/'bins'/f'{variant}-{mode}'
        shutil.copy2(target/'release/indexed-topk-diagnostic',binary)
        record = dict(variant=variant,mode=mode,command=command,exit_code=proc.returncode,seconds=time.time()-before,head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=wt,text=True).strip(),runtime_sha256=hashlib.sha256((wt/'crates/sparq-engine/src/exec.rs').read_bytes()).hexdigest(),binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),dirty_eager_control=variant=='eager')
        if mode == 'count':
            calibration = subprocess.run([str(binary),'selftest'],capture_output=True,text=True,env=env)
            (out/f'calibration-{variant}.txt').write_text(calibration.stdout+calibration.stderr)
            assert calibration.returncode == 0
        records.append(record)
        (out/'build-results.json').write_text(json.dumps(dict(records=records,head=head,total_seconds=time.time()-started,build_jobs=2,rayon_threads=1),indent=2)+'\n')
        print(json.dumps(record),flush=True)
finally:
    source.write_text(original)
    assert not subprocess.check_output(['git','status','--porcelain'],cwd=new,text=True)
    print('Committed source restored; all original binaries untouched',flush=True)
