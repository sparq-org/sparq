import datetime, hashlib, json, os, signal, shutil, subprocess, time
from pathlib import Path
P=Path(__file__).resolve().parent
R=P.parents[1]
W=R/'worktrees/issue6485'
FLOOR=2147483648
LIMIT=268435456

def stamp(): return datetime.datetime.now(datetime.timezone.utc).isoformat()
def write(n,x): (P/n).write_text(json.dumps(x,indent=2)+'\n')
def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def resources(): return {'free':shutil.disk_usage(P).free,'allocated':sum(f.stat().st_blocks*512 for f in P.rglob('*') if f.is_file())}
def main():
 initial=resources();assert initial['free']>=2214592512 and initial['allocated']<LIMIT,initial
 source=json.loads((P/'source-proof.json').read_text());assert sha(W/'crates/sparq-core/src/lib.rs')==source['current_lib_sha256']
 compiler=json.loads((P/'compiler-proof.json').read_text());assert sha(compiler['compiler'])==compiler['sha256']
 for lib in json.loads((P/'reused-artifacts.json').read_text()):assert sha(lib['path'])==lib['sha256']
 env=os.environ.copy()
 for key in list(env):
  if key.startswith('SPARQ_') or key in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUST_TEST_THREADS','CLIPPY_ARGS']:del env[key]
 env.update(json.loads((P/'recorded-environment.json').read_text()))
 env.update(CARGO_MANIFEST_DIR=str(W/'crates/sparq-core'),CARGO_MANIFEST_PATH=str(W/'crates/sparq-core/Cargo.toml'),CARGO_NET_OFFLINE='true',CARGO_INCREMENTAL='0',CARGO_BUILD_JOBS='1',RAYON_NUM_THREADS='1',RUST_BACKTRACE='0',TMPDIR=str(P/'tmp'))
 write('preexecution.json',{'at':stamp(),'initial':initial,'selected_environment':{k:env[k] for k in env if k.startswith('CARGO_') or k in ['RAYON_NUM_THREADS','RUST_BACKTRACE','TMPDIR']},'compiler_externs_source_rehashed':True})
 started=time.monotonic();deadline=started+600;receipts=[];samples=[]
 def run(name,argv):
  now=resources();assert now['free']>=FLOOR and now['allocated']<=LIMIT,now
  rec={'name':name,'argv':argv,'cwd':str(W),'started':stamp(),'before':now};write('logs/'+name+'-command.json',rec);t=time.monotonic();stop=None
  with (P/'logs'/f'{name}.stdout').open('w') as out,(P/'logs'/f'{name}.stderr').open('w') as err:
   proc=subprocess.Popen(argv,cwd=W,env=env,stdout=out,stderr=err,start_new_session=True);rec['pid']=proc.pid;last=0
   while proc.poll() is None:
    m=time.monotonic();r=resources();samples.append({'phase':name,'elapsed':m-started,**r})
    if r['free']<FLOOR:stop='free-floor'
    elif r['allocated']>LIMIT:stop='allocated-limit'
    elif m>=deadline:stop='time-limit'
    if stop:
     os.killpg(proc.pid,signal.SIGTERM)
     try:proc.wait(timeout=5)
     except subprocess.TimeoutExpired:os.killpg(proc.pid,signal.SIGKILL);proc.wait()
     break
    if m-last>=30:print(json.dumps({'phase':name,'elapsed':m-started,**r}),flush=True);last=m
    time.sleep(.5)
   code=proc.wait()
  rec.update(exit_code=code,stop=stop,seconds=time.monotonic()-t,ended=stamp(),after=resources(),terminal=True);receipts.append(rec);write('logs/'+name+'-receipt.json',rec);write('resources.json',samples);print(json.dumps({'name':name,'exit':code,'seconds':rec['seconds'],'stop':stop}),flush=True)
  return code,stop
 result='not-started';outcomes=[]
 # The documented Bash3 privacy failure is evidence, not a waived gate.
 pre=run('preflight',['python3','scripts/preflight.py','--base','HEAD'])
 write('preflight-result.json',{'exit':pre[0],'stop':pre[1],'note':'Inspect raw output; known local Bash3/mapfile limitation must remain explicit.'})
 if pre[1]:result='resource-stop'
 else:
  for spec in json.loads((P/'planned-commands.json').read_text()):
   name=spec['name'];code,stop=run('compile-'+name,spec['argv'])
   if code or stop:result='compile-or-resource-failure:'+name;break
   binary=Path(spec['binary']);write('artifacts/'+name+'/binary.json',{'path':str(binary),'sha256':sha(binary),'bytes':binary.stat().st_size})
   code,stop=run('test-'+name,[str(binary),'has_high_precision_decimal_memo','--nocapture','--test-threads=1'])
   text=(P/'logs'/f'test-{name}.stdout').read_text();valid=code==spec['expected_test_exit'] and not stop
   if spec['expected_pass_count'] is not None:valid=valid and f"test result: ok. {spec['expected_pass_count']} passed; 0 failed;" in text
   else:valid=valid and 'test result: FAILED.' in text and 'has_high_precision_decimal_memo' in text
   outcomes.append({'name':name,'exit':code,'expected_exit':spec['expected_test_exit'],'expected_result_observed':valid})
   write('outcomes.json',outcomes)
   if not valid:result='unexpected-behavior:'+name;break
  else:result='candidate-and-controls-complete'
 write('phase-result.json',{'result':result,'outcomes':outcomes,'receipts':receipts,'elapsed':time.monotonic()-started,'final_resources':resources(),'commands_pending':False,'retries':0})
 print(json.dumps({'result':result,'pending':False}),flush=True)
try:main()
except Exception as e:
 write('runner-error.json',{'error':repr(e),'at':stamp(),'resources':resources()});raise
