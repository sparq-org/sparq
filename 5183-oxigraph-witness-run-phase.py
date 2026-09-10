# [GPT-6 ASTRA] Bounded serial offline reference build and fixed run.
from pathlib import Path
import os, sys, time, json, subprocess, shutil, signal, re, hashlib
OUT=Path(__file__).resolve().parent
ROOT=OUT.parents[1]
WT=ROOT/'worktrees/issue5183'
TARGET=ROOT/'direct-5983/implementation/target'
TOOL=Path('/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin')
CARGO=str(TOOL/'cargo')
FLOOR=2*1024**3
LIMIT=512*1024**2
START=time.monotonic()
records=[]; resources=[]
def allocated(path):
 total=0
 for directory, _, files in os.walk(path):
  for name in files:
   try: total+=(Path(directory)/name).stat().st_blocks*512
   except FileNotFoundError: pass
 return total
INITIAL=allocated(TARGET)
def resource():
 target=allocated(TARGET); output=allocated(OUT); free=shutil.disk_usage(OUT).free
 value=dict(elapsed_seconds=time.monotonic()-START,target_bytes=target,output_bytes=output,aggregate_new_bytes=max(0,target-INITIAL)+output,free_bytes=free)
 resources.append(value)
 if free<FLOOR or value['aggregate_new_bytes']>LIMIT-8*1024**2 or value['elapsed_seconds']>=600:
  raise RuntimeError('Resource/time limit: '+json.dumps(value))
 return value
def save(name,obj): (OUT/name).write_text(json.dumps(obj,indent=2)+'\n')
env=os.environ.copy(); env.update(CARGO_PROFILE_RELEASE_OPT_LEVEL='3',CARGO_PROFILE_RELEASE_DEBUG='false',CARGO_PROFILE_RELEASE_LTO='false',CARGO_PROFILE_RELEASE_CODEGEN_UNITS='16',CARGO_PROFILE_RELEASE_PANIC='unwind',CARGO_BUILD_JOBS='1',CARGO_INCREMENTAL='0',CARGO_NET_OFFLINE='true',CARGO_TARGET_DIR=str(TARGET),RUSTC=str(TOOL/'rustc'),RAYON_NUM_THREADS='1')
env['PATH']=str(TOOL)+':'+env['PATH']
controlled={k:env[k] for k in ['CARGO_PROFILE_RELEASE_OPT_LEVEL','CARGO_PROFILE_RELEASE_DEBUG','CARGO_PROFILE_RELEASE_LTO','CARGO_PROFILE_RELEASE_CODEGEN_UNITS','CARGO_PROFILE_RELEASE_PANIC','CARGO_BUILD_JOBS','CARGO_INCREMENTAL','CARGO_NET_OFFLINE','CARGO_TARGET_DIR','RUSTC','RAYON_NUM_THREADS']}
def run(name,argv):
 resource(); started=time.monotonic(); record={'name':name,'argv':argv,'cwd':str(OUT/'harness'),'env':controlled,'started_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())}
 records.append(record); save('commands.json',records)
 with open(OUT/(name+'.stdout'),'wb') as stdout,open(OUT/(name+'.stderr'),'wb') as stderr:
  proc=subprocess.Popen(argv,cwd=OUT/'harness',env=env,stdout=stdout,stderr=stderr,start_new_session=True)
  last=0
  try:
   while proc.poll() is None:
    receipt=resource()
    if time.monotonic()-last>25:
     print(name,json.dumps(receipt),flush=True); last=time.monotonic()
    time.sleep(2)
  except BaseException:
   os.killpg(proc.pid,signal.SIGTERM)
   try: proc.wait(timeout=5)
   except subprocess.TimeoutExpired: os.killpg(proc.pid,signal.SIGKILL); proc.wait()
   record.update(exit=proc.returncode,stopped=True,elapsed_seconds=time.monotonic()-started); save('commands.json',records)
   raise
 record.update(exit=proc.returncode,elapsed_seconds=time.monotonic()-started); save('commands.json',records)
 if proc.returncode: raise RuntimeError(name+' exit '+str(proc.returncode))
 resource()
def pins(text):
 result={}
 for block in text.split('[[package]]')[1:]:
  fields={key:re.search(r'^'+key+r' = "([^\"]+)"',block,re.M) for key in ['name','version','source','checksum']}
  p={key:value.group(1) if value else None for key,value in fields.items()}
  result[(p['name'],p['version'])]=p
 return result
status={'status':'running','initial_target_allocated_bytes':INITIAL}
try:
 resource(); save('initial-resource.json',resources[-1])
 version=subprocess.check_output([str(TOOL/'rustc'),'-Vv'],text=True); (OUT/'rustc-version.txt').write_text(version)
 run('metadata',[CARGO,'metadata','--offline','--format-version','1','--filter-platform','aarch64-apple-darwin'])
 metadata=json.loads((OUT/'metadata.stdout').read_text()); rootpins=pins((OUT/'repository-Cargo.lock').read_text()); resolvedpins=pins((OUT/'harness/Cargo.lock').read_text())
 changes=[]
 for key,p in resolvedpins.items():
  if p['name']=='issue5183-oxigraph-witness': continue
  if rootpins.get(key)!=p: changes.append({'resolved':p,'repository':rootpins.get(key)})
 assert not changes,changes
 resolved={node['id']:node['features'] for node in metadata['resolve']['nodes']}
 oxi=next(p for p in metadata['packages'] if p['name']=='oxigraph'); features=resolved[oxi['id']]
 assert features==['rdf-12'],features
 assert not any(p['name'] in ['oxrocksdb-sys','oxhttp'] for p in metadata['packages'])
 parser=next(p for p in metadata['packages'] if p['name']=='spargebra')
 assert parser['version']=='0.4.6' and Path(parser['manifest_path'])==WT/'vendor/spargebra/Cargo.toml'
 save('resolved-proof.json',{'dependency_pin_drift':changes,'resolved_package_count':len(metadata['packages']),'oxigraph_features':features,'spargebra_manifest':parser['manifest_path'],'all_features':resolved,'resolved_lock_sha256':hashlib.sha256((OUT/'harness/Cargo.lock').read_bytes()).hexdigest()})
 run('build',[CARGO,'build','--release','--locked','--offline','-j','1','-vv'])
 binary=TARGET/'release/issue5183-oxigraph-witness'
 shutil.copy2(binary,OUT/'oxigraph-witness')
 save('binary.json',{'path':'oxigraph-witness','sha256':hashlib.sha256((OUT/'oxigraph-witness').read_bytes()).hexdigest(),'bytes':(OUT/'oxigraph-witness').stat().st_size})
 run('fixed-cases',[str(OUT/'oxigraph-witness')])
 observations=[json.loads(line) for line in (OUT/'fixed-cases.stdout').read_text().splitlines()]
 assert [v['case'] for v in observations]==['lexical-pair','canonical-eight','different-values']
 save('observations.json',observations)
 status.update(status='complete',cases=len(observations))
except BaseException as error:
 status.update(status='stopped',error=repr(error))
 print('STOP',repr(error),flush=True)
finally:
 status['elapsed_seconds']=time.monotonic()-START
 save('resources.json',resources); save('phase-result.json',status)
 print(json.dumps(status),flush=True)
