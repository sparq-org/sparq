from pathlib import Path
import subprocess,json,os,time
root=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246')
out=Path(__file__).parent
env=os.environ.copy();env.update(RAYON_NUM_THREADS='2',CARGO_BUILD_JOBS='2',CARGO_TARGET_DIR='/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
cargo='/Users/jesght/.cargo/bin/cargo'
commands=[('store-default',[cargo,'test','--locked','--offline','-p','sparq-core','--lib','store::','--','--test-threads=1']),('store-compact',[cargo,'test','--locked','--offline','-p','sparq-core','--no-default-features','--features','compact-index','--lib','store::','--','--test-threads=1']),('snapshot-fork',[cargo,'test','--locked','--offline','-p','sparq-core','--test','snapshot','--test','fork_differential','--','--test-threads=1']),('clippy-core',[cargo,'clippy','--locked','--offline','-p','sparq-core','--lib','--','-D','warnings']),('clippy-harness',[cargo,'clippy','--locked','--offline','--manifest-path','bench/overlay-count/Cargo.toml','--features','count-alloc','--','-D','warnings']),('preflight',['python3','scripts/preflight.py','--base','a42a9e89dec485f6a319c47cb3635c59cb5a2270'])]
results=[]
for name,cmd in commands:
 start=time.monotonic()
 with (out/(name+'.log')).open('w') as log:r=subprocess.run(cmd,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=300)
 results.append(dict(name=name,command=cmd,exit=r.returncode,seconds=time.monotonic()-start))
 (out/'results.json').write_text(json.dumps(results,indent=2)+'\n')
 print(results[-1],flush=True)
