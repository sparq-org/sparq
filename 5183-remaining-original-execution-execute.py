from pathlib import Path
import os,json,subprocess,time,signal,hashlib,sys
p=Path(__file__).resolve().parent;r=p.parent.parent;old=p.parent/'original-seed-execution';target=r/'direct-5983/implementation/target'
identity=json.loads((p/'readiness-identities-and-cache.json').read_text());protocol=json.loads((p/'protocol.json').read_text());expected=json.loads((p/'initial-provenance.json').read_text());sha=lambda b:hashlib.sha256(b).hexdigest()
assert not (p/'commands.json').exists(),'No repeat execution'
def allocated(q):return sum(x.stat().st_blocks*512 for x in q.rglob('*') if x.is_file() and not x.is_symlink())
initial_target=allocated(target);start=time.monotonic();commands=[];resources=[];proofs=[];binaries={}
def save(n,v):(p/n).write_text(json.dumps(v,indent=2)+'\n')
def check():
 free=os.statvfs(p).f_bavail*os.statvfs(p).f_frsize;now=time.monotonic()-start;outputs=allocated(p);growth=outputs+max(0,allocated(target)-initial_target)
 value={'elapsed_seconds':now,'free_bytes':free,'output_allocated':outputs,'new_allocated':growth};resources.append(value);save('resources.json',resources)
 assert free>=2147483648,'free disk floor';assert growth<130023424,'new allocation bound with4MiB export reserve';assert now<600,'aggregate time bound'
 return value
def verify(rev):
 q=p/rev/'harness/crates/sparq-bench';source=p/'source/original-update_fuzz.rs';assert sha(source.read_bytes())==protocol['source_module_sha256']
 rustc=Path(json.loads((p/(rev+'-compiler-argv.json')).read_text())[0]);assert sha(rustc.read_bytes())==expected['compiler_sha256']
 deps=[]
 for d in identity['direct_link_dependencies'][rev]:
  h=sha(Path(d['path']).read_bytes());assert h==d['expected_sha256'];deps.append({'crate':d['crate'],'path':d['path'],'sha256':h})
 allow=q/'../../bench/differential-divergences.json';assert sha(allow.read_bytes())==identity['frozen_binaries'][rev]['allowlist_sha256']
 proof={'revision':rev,'seconds':time.monotonic()-start,'source':str(source),'source_sha256':sha(source.read_bytes()),'driver_sha256':sha((q/'main.rs').read_bytes()),'compiler_sha256':expected['compiler_sha256'],'externs':deps,'allowlist_sha256':sha(allow.read_bytes()),'allowlist_resolved':str(allow.resolve())};proofs.append(proof);save('precommand-provenance.json',proofs)
 return q
def command(name,argv,rev,limit):
 check();q=verify(rev);env={k:v for k,v in os.environ.items() if not k.startswith(('SPARQ_','CARGO_PROFILE_')) and k not in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS')}
 controlled=dict(json.loads((old/'environment-policy.json').read_text())['controlled']);controlled.update(TMPDIR=str(p/'tmp'),CARGO_MANIFEST_DIR=str(q));env.update(controlled)
 row={'name':name,'argv':argv,'cwd':str(q),'controlled_environment':controlled,'SPARQ_FUZZ_DIVERGENCES':'absent','started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())};t=time.monotonic();stopped=None
 with (p/(name+'.stdout')).open('wb') as out,(p/(name+'.stderr')).open('wb') as err:
  proc=subprocess.Popen(argv,cwd=q,env=env,stdout=out,stderr=err,start_new_session=True)
  last=t
  while proc.poll() is None:
   try:
    check();assert time.monotonic()-t<limit,'per-command time bound'
   except Exception as e:
    stopped=str(e);os.killpg(proc.pid,signal.SIGTERM)
    try:proc.wait(timeout=4)
    except subprocess.TimeoutExpired:os.killpg(proc.pid,signal.SIGKILL);proc.wait()
    break
   if time.monotonic()-last>=30:print(name,'running',round(time.monotonic()-t,1),flush=True);last=time.monotonic()
   time.sleep(.35)
  code=proc.wait()
 row.update(exit=code,seconds=time.monotonic()-t,stopped=stopped);commands.append(row);save('commands.json',commands);check();print(json.dumps({k:row[k] for k in ('name','exit','seconds','stopped')}),flush=True)
 assert stopped is None,stopped
 return code
save('initial-resource.json',{'target_allocated':initial_target,'free_bytes':os.statvfs(p).f_bavail*os.statvfs(p).f_frsize})
try:
 for rev in ['parent','main']:
  argv=json.loads((p/(rev+'-compiler-argv.json')).read_text());code=command(rev+'-build',argv,rev,120);assert code==0,'driver build failed'
  q=p/rev/'harness/crates/sparq-bench';binary=q/'out'/('issue5183_remaining_original_'+rev+'-'+rev);assert binary.is_file(),binary
  binaries[rev]={'path':str(binary),'sha256':sha(binary.read_bytes()),'bytes':binary.stat().st_size};save('binaries.json',binaries)
 for rev in ['parent','main']:
  for seed in protocol['seeds']:
   b=binaries[rev];assert sha(Path(b['path']).read_bytes())==b['sha256'];name=rev+'-seed-'+str(seed)
   code=command(name,[b['path'],str(seed)],rev,30);assert code in (0,1),'unexpected driver exit'
   text=(p/(name+'.stdout')).read_text();assert 'adjudicated classes enabled ["update-oxigraph-integer-lexical-canonicalization"]; every other divergence fails' in text,'default allowlist startup not established'
 save('phase-result.json',{'status':'complete','commands':len(commands),'seed_runs':14,'all_returned':True,'seconds':time.monotonic()-start,'last_resource':check()});print('ALL16 COMMANDS TERMINAL',flush=True)
except Exception as e:
 save('phase-result.json',{'status':'stopped','reason':str(e),'commands':len(commands),'seconds':time.monotonic()-start,'all_started_commands_returned':True});print('STOP',e,flush=True);sys.exit(1)
