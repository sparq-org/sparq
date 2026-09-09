from pathlib import Path
import os,subprocess,time,json,shutil,hashlib
p=Path(__file__).resolve().parents[1]
base=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees')
roots={'candidate':base/'issue4246','main':base/'issue4246-optin-main'}
target=Path('/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/diagnostic/target')
env={**os.environ,'RAYON_NUM_THREADS':'1','CARGO_BUILD_JOBS':'2','CARGO_TARGET_DIR':str(target)}
records=[];started=time.monotonic()
for source,allocation in [('candidate',False),('candidate',True),('main',False),('main',True)]:
 name=source+('-count' if allocation else '-time')
 flags=[]
 if source=='candidate':flags.append('sparq-core/overlay-deleted-projections')
 if allocation:flags.append('count-alloc')
 cmd=['/Users/jesght/.cargo/bin/cargo','build','--locked','--offline','--release','--manifest-path','bench/overlay-count/Cargo.toml']
 if flags:cmd+=['--features',','.join(flags)]
 remain=900-(time.monotonic()-started)
 assert remain>0,'15 minute optimized build cap'
 begin=time.time()
 with (p/'builds'/(name+'.log')).open('w') as log:
  r=subprocess.run(cmd,cwd=roots[source],env=env,stdout=log,stderr=subprocess.STDOUT,timeout=remain)
 assert r.returncode==0,(name,r.returncode)
 dest=p/'binaries'/name;shutil.copy2(target/'release/overlay-count-diagnostic',dest)
 records.append(dict(name=name,command=cmd,head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=roots[source],text=True).strip(),started=begin,ended=time.time(),sha256=hashlib.sha256(dest.read_bytes()).hexdigest(),bytes=dest.stat().st_size))
 (p/'builds/results.json').write_text(json.dumps(records,indent=2)+'\n')
 print(records[-1],flush=True)
print('total_build_wall_seconds',time.monotonic()-started,flush=True)
