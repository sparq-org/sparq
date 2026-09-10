# GPT-6 Astra: bounded exact in-tree Cargo tests; no retry or cleanup.
import os,sys,json,time,shutil,subprocess,signal
from pathlib import Path
v=Path(__file__).resolve().parent;p=json.loads((v/'protocol.json').read_text());t=Path(p['target']);old=[Path(x) for x in p['prior_evidence']]
def allocated(root):
 total=0
 for x in [root,*root.rglob('*')]:
  try:
   if not x.is_symlink(): total+=x.stat().st_blocks*512
  except FileNotFoundError:
   pass # Rustc may remove its own temporary object between enumeration and stat.
 return total
def measure():
 return shutil.disk_usage('/private/tmp').free,allocated(t)+sum(allocated(x) for x in old)+allocated(v)-p['original_target_initial_allocated_bytes']
label=sys.argv[1];cmd=sys.argv[2:];before=measure();assert before[0]>=p['minimum_free_bytes'] and before[1]<p['aggregate_growth_limit_bytes'],before
assert time.time()-p['started_epoch']<p['max_seconds']
env=os.environ.copy();env.update(p['profile_overrides']);env.update(CARGO_BUILD_JOBS='1',CARGO_INCREMENTAL='0',CARGO_NET_OFFLINE='true',CARGO_TARGET_DIR=str(t),RUSTC='/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc')
start=time.time();stop=None;maxgrowth=before[1];minfree=before[0]
with (v/(label+'.log')).open('w') as log:
 proc=subprocess.Popen(cmd,stdout=log,stderr=subprocess.STDOUT,env=env,start_new_session=True)
 while proc.poll() is None:
  free,growth=measure();minfree=min(minfree,free);maxgrowth=max(maxgrowth,growth)
  if free<p['minimum_free_bytes'] or growth>=p['aggregate_growth_limit_bytes'] or time.time()-p['started_epoch']>=p['max_seconds']:
   stop='disk-free' if free<p['minimum_free_bytes'] else 'aggregate-growth' if growth>=p['aggregate_growth_limit_bytes'] else 'phase-time';os.killpg(proc.pid,signal.SIGTERM);proc.wait();break
  time.sleep(.5)
after=measure();r={'label':label,'command':cmd,'start_epoch':start,'seconds':time.time()-start,'exit_code':proc.returncode,'stop':stop,'before_free_growth':before,'after_free_growth':after,'max_growth_bytes':max(maxgrowth,after[1]),'min_free_bytes':min(minfree,after[0]),'env':{k:env[k] for k in list(p['profile_overrides'])+['CARGO_BUILD_JOBS','CARGO_INCREMENTAL','CARGO_NET_OFFLINE','CARGO_TARGET_DIR','RUSTC']}}
(v/(label+'.json')).write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r));print('\n'.join((v/(label+'.log')).read_text().splitlines()[-12:]));sys.exit(0 if proc.returncode==0 else 1)
