from pathlib import Path
import os,errno,stat,subprocess,time,signal,json,hashlib,shutil,sys
p=Path(__file__).resolve().parent;r=p.parent.parent;w=r/'worktrees/issue6483';ready=p.parent/'readiness'
started=time.monotonic();commands=[];resources=[];disappeared=0
sha=lambda b:hashlib.sha256(b).hexdigest()
def put(n,v):(p/n).write_text(json.dumps(v,indent=2)+'\n' if not isinstance(v,str) else v)
def enoent_only(e):
 global disappeared
 if e.errno!=errno.ENOENT:raise e
 disappeared+=1
def check(initial=False):
 total=0
 for parent,dirs,files in os.walk(p,onerror=enoent_only,followlinks=False):
  for name in files:
   try:
    s=(Path(parent)/name).lstat()
    if not stat.S_ISLNK(s.st_mode):total+=s.st_blocks*512
   except OSError as e:enoent_only(e)
 row={'seconds':time.monotonic()-started,'allocated_bytes':total,'free_bytes':shutil.disk_usage(p).free,'enoent_entries':disappeared};resources.append(row);put('resources.json',resources)
 assert row['free_bytes']>=(2214592512 if initial else 2147483648),'free floor'
 assert total<67108864-4194304,'allocated cap less export reserve'
 assert row['seconds']<120,'aggregate120s'
 return row
def run(name,argv,limit):
 check();start=time.monotonic();stopped=None
 with (p/(name+'.stdout')).open('wb') as out,(p/(name+'.stderr')).open('wb') as err:
  proc=subprocess.Popen(argv,cwd=p,env=env,stdout=out,stderr=err,start_new_session=True)
  while proc.poll() is None:
   try:check();assert time.monotonic()-start<limit,'command timeout'
   except Exception as exc:
    stopped=str(exc);os.killpg(proc.pid,signal.SIGTERM)
    try:proc.wait(timeout=2)
    except subprocess.TimeoutExpired:os.killpg(proc.pid,signal.SIGKILL);proc.wait()
    break
   time.sleep(.2)
  code=proc.wait()
 record={'name':name,'argv':argv,'exit':code,'seconds':time.monotonic()-start,'stop_reason':stopped};commands.append(record);put('commands.json',commands);print(json.dumps(record),flush=True)
 check();assert code==0 and stopped is None,'unexpected tool/control failure; no retry'
try:
 check(True)
 art=json.loads((ready/'warm-artifacts.json').read_text());src=json.loads((ready/'source-proof.json').read_text())
 for x in [art['compiler']]+art['direct_libraries_rehashed']:assert sha(Path(x['path']).read_bytes())==x['sha256']
 for x in src['selected_sources']:assert sha((w/x['path']).read_bytes())==x['sha256']
 for x in src['recorded_comparison']:assert sha((w/x['path']).read_bytes())==x['current_sha256']
 assert not subprocess.check_output(['git','status','--porcelain'],cwd=w)
 assert not subprocess.check_output(['git','diff',src['known_main'],'HEAD','--','crates/sparq-engine','crates/sparq-core','Cargo.lock'],cwd=w)
 manifest=json.loads((ready/'manifest.json').read_text())
 for x in manifest['files']:assert sha((ready/x['path']).read_bytes())==x['sha256']
 libs={}
 for x in art['direct_libraries_rehashed']:
  n=Path(x['path']).name.split('-')[0][3:];libs[n]=x['path']
 controlled={'RAYON_NUM_THREADS':'1','CARGO_BUILD_JOBS':'1','CARGO_INCREMENTAL':'0','CARGO_NET_OFFLINE':'true','RUST_BACKTRACE':'1','TMPDIR':str(p/'tmp')}
 (p/'tmp').mkdir()
 env={k:v for k,v in os.environ.items() if not k.startswith(('SPARQ_','CARGO_PROFILE_')) and k not in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS')};env.update(controlled)
 argv=[art['compiler']['path'],'--edition=2021','--crate-name','issue6479_public_base',str(p/'public_base.rs'),'--emit=link,dep-info','-C','opt-level=3','-C','embed-bitcode=no','-C','codegen-units=16','-C','debuginfo=0','-C','strip=debuginfo','-C','panic=unwind','-L','dependency='+str(Path(libs['sparq_engine']).parent)]
 for name in ['sparq_engine','sparq_core','oxrdf']:argv+=['--extern',name+'='+libs[name]]
 argv+=['-o',str(p/'public-base')]
 put('protocol.json',{'frozen_input':'direct-6479/readiness/witness-protocol.json','sha256':sha((ready/'witness-protocol.json').read_bytes()),'cases':5,'links':1,'processes':1,'source_sha256':sha((p/'public_base.rs').read_bytes()),'driver_profile':'Rust2021/O3/unwind/default-no-LTO/codegen16','env':controlled,'limits':json.loads((r/'direct-6479/admission/witness-admission.json').read_text())['limits'],'written_before_execution':True})
 put('provenance.json',{'compiler':art['compiler'],'libs':art['direct_libraries_rehashed'],'source_equivalence':src,'feature_profile':art['feature_profile'],'source_sha256':sha((p/'public_base.rs').read_bytes()),'rustc_argv':argv,'old_readiness_manifest_sha256':sha((ready/'manifest.json').read_bytes())})
 run('link',argv,60)
 put('binary.json',{'path':'public-base','sha256':sha((p/'public-base').read_bytes()),'bytes':(p/'public-base').stat().st_size})
 run('fixed-cases',[str(p/'public-base')],30)
 output=(p/'fixed-cases.stdout').read_text();assert output.count('callback_count=1 same_thread=true')==5;assert 'completed_fixed_cases=5' in output
 for x in manifest['files']:assert sha((ready/x['path']).read_bytes())==x['sha256']
 assert not subprocess.check_output(['git','status','--porcelain'],cwd=w)
 put('phase-result.json',{'status':'complete','seconds':time.monotonic()-started,'commands_terminal':len(commands),'pending':False,'readiness_unchanged':True,'worktree_clean':True})
except Exception as exc:
 put('phase-result.json',{'status':'stopped','reason':str(exc),'seconds':time.monotonic()-started,'commands_terminal':len(commands),'pending':False});print('STOP',str(exc),flush=True);sys.exit(1)
