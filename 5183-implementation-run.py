# [GPT-6 ASTRA] One-process scoped validation monitor; no command retries.
from pathlib import Path
import sys,os,json,time,subprocess,shutil,signal
P=Path(__file__).resolve().parent;R=P.parents[1];T=R/'direct-5983/implementation/target';H=P/'harness/crates/sparq-bench';TOOL=Path('/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin')
initial=json.loads((P/'initial-resource.json').read_text());records=json.loads((P/'commands.json').read_text()) if (P/'commands.json').exists() else [];resources=json.loads((P/'resources.json').read_text()) if (P/'resources.json').exists() else []
name,limit,*cmd=sys.argv[1:];limit=float(limit);assert not any(x['name']==name for x in records)
env=os.environ.copy()
for k in list(env):
 if k.startswith(('SPARQ_','CARGO_PROFILE_')) or k in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RAYON_NUM_THREADS']:env.pop(k)
env.update(CARGO_PROFILE_RELEASE_OPT_LEVEL='3',CARGO_PROFILE_RELEASE_DEBUG='false',CARGO_PROFILE_RELEASE_LTO='false',CARGO_PROFILE_RELEASE_CODEGEN_UNITS='16',CARGO_PROFILE_RELEASE_PANIC='unwind',CARGO_BUILD_JOBS='1',CARGO_INCREMENTAL='0',CARGO_NET_OFFLINE='true',CARGO_TARGET_DIR=str(T),RUSTC=str(TOOL/'rustc'),RUST_BACKTRACE='1',RAYON_NUM_THREADS='1',TMPDIR=str(P/'tmp'),CARGO_MANIFEST_DIR=str(H));env['PATH']=str(TOOL)+':'+env['PATH']
def save():
 (P/'commands.json').write_text(json.dumps(records,indent=2)+'\n');(P/'resources.json').write_text(json.dumps(resources,indent=2)+'\n')
def alloc(p):
 n=0
 for root,_,files in os.walk(p):
  for f in files:
   try:n+=(Path(root)/f).stat().st_blocks*512
   except FileNotFoundError:pass
 return n
start=time.monotonic();prior=sum(x.get('seconds',0) for x in records)
def check():
 d={'command':name,'seconds':time.monotonic()-start,'aggregate_seconds':prior+time.monotonic()-start,'free_bytes':shutil.disk_usage(P).free,'target_allocated':alloc(T),'output_allocated':alloc(P)};d['new_allocated']=max(0,d['target_allocated']-initial['initial_target_allocated'])+d['output_allocated'];resources.append(d)
 if d['free_bytes']<2*1024**3 or d['new_allocated']>504*1024**2 or d['aggregate_seconds']>1200 or d['seconds']>limit:raise RuntimeError('BOUND '+json.dumps(d))
 return d
check();rec={'name':name,'argv':cmd,'cwd':str(H),'started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'environment':{k:env[k] for k in env if k.startswith('CARGO_PROFILE_') or k in ['CARGO_BUILD_JOBS','CARGO_INCREMENTAL','CARGO_NET_OFFLINE','CARGO_TARGET_DIR','RUSTC','RUST_BACKTRACE','RAYON_NUM_THREADS','CARGO_MANIFEST_DIR','TMPDIR']}};records.append(rec);save()
with (P/(name+'.stdout')).open('wb') as out,(P/(name+'.stderr')).open('wb') as err:
 proc=subprocess.Popen(cmd,cwd=H,env=env,stdout=out,stderr=err,start_new_session=True);last=0
 try:
  while proc.poll() is None:
   d=check()
   if time.monotonic()-last>25: print(json.dumps(d),flush=True);last=time.monotonic()
   time.sleep(1)
 except BaseException as e:
  os.killpg(proc.pid,signal.SIGTERM)
  try:proc.wait(timeout=5)
  except subprocess.TimeoutExpired:os.killpg(proc.pid,signal.SIGKILL);proc.wait()
  rec['stopped']=str(e)
 rec['exit']=proc.wait();rec['seconds']=time.monotonic()-start;save()
print(json.dumps({'name':name,'exit':rec['exit'],'seconds':rec['seconds'],'stopped':rec.get('stopped')}),flush=True)
sys.exit(rec['exit'] if 'stopped' not in rec else 124)
