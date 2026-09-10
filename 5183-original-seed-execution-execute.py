# [GPT-6 ASTRA] Monitored one-seed comparison; preserve original source and failure exits.
from pathlib import Path, PurePosixPath
import os,sys,time,json,subprocess,shutil,signal,hashlib,re,tarfile
OUT=Path(__file__).resolve().parent;R=OUT.parents[1];WT=R/'worktrees/issue5183';TARGET=R/'direct-5983/implementation/target';TOOL=Path('/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin')
HEADS={'parent':'d41ec9fcb796504d85f5a5247a5fdc9eb3e65de5','main':'781f667c19a8ebb779cfccb24b05ea432360b025'}
START=time.monotonic();FLOOR=2*1024**3;CAP=512*1024**2;commands=[];resources=[]
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def save(n,d):(OUT/n).write_text(json.dumps(d,indent=2)+'\n')
def alloc(p):
 total=0
 for d,_,fs in os.walk(p):
  for n in fs:
   try:total+=(Path(d)/n).stat().st_blocks*512
   except FileNotFoundError:pass
 return total
INITIAL=alloc(TARGET)
def resource():
 target=alloc(TARGET);output=alloc(OUT);free=shutil.disk_usage(OUT).free
 d={'seconds':time.monotonic()-START,'target_allocated':target,'output_allocated':output,'new_allocated':max(0,target-INITIAL)+output,'free':free};resources.append(d)
 if free<FLOOR or d['new_allocated']>=CAP-8*1024**2 or d['seconds']>=1200:raise RuntimeError('Resource bound '+json.dumps(d))
 return d
ENV=os.environ.copy();removed=[]
for key in list(ENV):
 if key.startswith('SPARQ_') or key.startswith('CARGO_PROFILE_') or key in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RAYON_NUM_THREADS']:
  removed.append(key);ENV.pop(key)
ENV.update(CARGO_PROFILE_RELEASE_OPT_LEVEL='3',CARGO_PROFILE_RELEASE_DEBUG='false',CARGO_PROFILE_RELEASE_LTO='false',CARGO_PROFILE_RELEASE_CODEGEN_UNITS='16',CARGO_PROFILE_RELEASE_PANIC='unwind',CARGO_BUILD_JOBS='1',CARGO_INCREMENTAL='0',CARGO_NET_OFFLINE='true',CARGO_TARGET_DIR=str(TARGET),RUSTC=str(TOOL/'rustc'),RUST_BACKTRACE='1',RAYON_NUM_THREADS='1')
ENV['PATH']=str(TOOL)+':'+ENV['PATH']
(OUT/'tmp').mkdir();ENV['TMPDIR']=str(OUT/'tmp')
controlled={k:v for k,v in ENV.items() if k.startswith('CARGO_PROFILE_') or k in ['CARGO_BUILD_JOBS','CARGO_INCREMENTAL','CARGO_NET_OFFLINE','CARGO_TARGET_DIR','RUSTC','RUST_BACKTRACE','RAYON_NUM_THREADS','TMPDIR']}
def run(name,argv,cwd,limit,accepted=(0,)):
 resource();t=time.monotonic();rec={'name':name,'argv':argv,'cwd':str(cwd),'env':controlled,'start_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())};commands.append(rec);save('commands.json',commands)
 with open(OUT/(name+'.stdout'),'wb') as so,open(OUT/(name+'.stderr'),'wb') as se:
  proc=subprocess.Popen(argv,cwd=cwd,env=ENV,stdout=so,stderr=se,start_new_session=True);last=0
  try:
   while proc.poll() is None:
    d=resource()
    if time.monotonic()-t>=limit:raise RuntimeError(name+' time bound')
    if time.monotonic()-last>=25:print(name,json.dumps(d),flush=True);last=time.monotonic()
    time.sleep(2)
  except BaseException:
   os.killpg(proc.pid,signal.SIGTERM)
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:os.killpg(proc.pid,signal.SIGKILL);proc.wait()
   rec.update(exit=proc.returncode,stopped=True,seconds=time.monotonic()-t);save('commands.json',commands);raise
 rec.update(exit=proc.returncode,seconds=time.monotonic()-t);save('commands.json',commands)
 if proc.returncode not in accepted:raise RuntimeError(name+' exit '+str(proc.returncode))
 resource();return proc.returncode

def pins(text):
 records={}
 for block in text.split('[[package]]')[1:]:
  row={}
  for k in ['name','version','source','checksum']:
   m=re.search('^'+k+r' = "([^\"]+)"',block,re.M);row[k]=m.group(1) if m else None
  records[(row['name'],row['version'])]=row
 return records

def main_clean():
 assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=WT,text=True).strip()==HEADS['main']
 assert subprocess.check_output(['git','status','--porcelain'],cwd=WT,text=True)==''

def source_record(root):
 paths=[]
 for rel in ['crates/sparq-core','crates/sparq-engine','crates/sparq-canon','crates/sparq-substrate','vendor/spargebra']:
  for f in sorted((root/rel).rglob('*')):
   if f.is_file() and not any(part in ['target','.git'] for part in f.relative_to(root).parts):paths.append({'path':str(f.relative_to(root)),'bytes':f.stat().st_size,'sha256':sha(f)})
 for rel in ['Cargo.toml','Cargo.lock','crates/sparq-bench/src/update_fuzz.rs','crates/sparq-bench/Cargo.toml','bench/differential-divergences.json','.github/workflows/differential-update.yml']:
  f=root/rel;paths.append({'path':rel,'bytes':f.stat().st_size,'sha256':sha(f)})
 return paths

status={'status':'running','initial_target_allocated':INITIAL,'start_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'deadline_seconds':1200}
try:
 main_clean();resource();save('initial-resource.json',resources[-1]);save('environment-policy.json',{'controlled':controlled,'removed_key_names_only':removed,'SPARQ_FUZZ_DIVERGENCES':'absent/default manifest-relative allowlist','runtime_thread_limit':'RAYON_NUM_THREADS=1 diagnostic scope, not exact Linux runner thread parity'})
 (OUT/'rustc-version.txt').write_text(subprocess.check_output([str(TOOL/'rustc'),'-Vv'],text=True))
 parent=OUT/'parent-source';parent.mkdir();archive_start=time.monotonic();count=0
 argv=['git','archive','--format=tar',HEADS['parent']]
 rec={'name':'parent-source-export','argv':argv,'cwd':str(WT),'start_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())};commands.append(rec)
 with open(OUT/'parent-source-export.stderr','wb') as se:
  proc=subprocess.Popen(argv,cwd=WT,stdout=subprocess.PIPE,stderr=se,start_new_session=True)
  try:
   with tarfile.open(fileobj=proc.stdout,mode='r|') as tf:
    for member in tf:
     rel=PurePosixPath(member.name);assert not rel.is_absolute() and '..' not in rel.parts
     dest=parent/member.name
     if member.isdir():dest.mkdir(parents=True,exist_ok=True)
     elif member.isfile():
      dest.parent.mkdir(parents=True,exist_ok=True)
      with tf.extractfile(member) as src,open(dest,'wb') as dst:shutil.copyfileobj(src,dst)
      os.chmod(dest,member.mode&0o777);count+=1
     else:raise RuntimeError('Unexpected archive member '+member.name)
     if count%200==0:resource()
   code=proc.wait(timeout=10)
  except BaseException:
   os.killpg(proc.pid,signal.SIGTERM);proc.wait(timeout=5);raise
 rec.update(exit=code,seconds=time.monotonic()-archive_start,files=count);save('commands.json',commands);assert code==0
 listing=subprocess.check_output(['git','ls-tree','-r',HEADS['parent']],cwd=WT,text=True);exportproof=[]
 for line in listing.splitlines():
  left,name=line.split('\t',1);mode,kind,oid=left.split();assert kind=='blob'
  content=(parent/name).read_bytes();actual=hashlib.sha1(b'blob '+str(len(content)).encode()+b'\0'+content).hexdigest();assert actual==oid,name
  exportproof.append({'path':name,'git_blob':oid,'sha256':hashlib.sha256(content).hexdigest(),'bytes':len(content)})
 save('parent-export-proof.json',{'head':HEADS['parent'],'files':exportproof});resource()
 expected_features=json.loads((R/'direct-5183/original-seed-readiness/feature-equivalence.json').read_text())['minimal_features']
 outcomes=[]
 for variant,source in [('parent',parent),('main',WT)]:
  main_clean();before=source_record(source);save(variant+'-source-before.json',before)
  harness=OUT/variant/'harness/crates/sparq-bench';(harness/'src').mkdir(parents=True);allow=harness.parent.parent/'bench/differential-divergences.json';allow.parent.mkdir();shutil.copyfile(source/'bench/differential-divergences.json',allow)
  module=source/'crates/sparq-bench/src/update_fuzz.rs';code='// [GPT-6 ASTRA] One original fixed seed through the unchanged differential module.\n#[path = '+json.dumps(str(module))+']\nmod update_fuzz;\nfn main() { update_fuzz::run(4141222487, 1); }\n';(harness/'src/main.rs').write_text(code)
  name='issue5183-original-'+variant
  manifest='[package]\nname='+json.dumps(name)+'\nversion="0.0.0"\nedition="2024"\n[workspace]\nresolver="2"\n[dependencies]\n'
  for crate,features in [('sparq-core',['mmap','dict-spill']),('sparq-engine',['algebra-rewrite']),('sparq-canon',['rdf12-triple-terms'])]:manifest+=crate+'={path='+json.dumps(str(source/'crates'/crate))+',features='+json.dumps(features)+'}\n'
  manifest+='oxigraph={version="=0.5.9",default-features=false,features=["rdf-12"]}\noxrdf={version="=0.3.3",features=["rdf-12"]}\nserde_json="1"\n[patch.crates-io]\nspargebra={path='+json.dumps(str(source/'vendor/spargebra'))+'}\n';(harness/'Cargo.toml').write_text(manifest);shutil.copyfile(source/'Cargo.lock',harness/'Cargo.lock')
  assert (harness/'../../bench/differential-divergences.json').resolve()==allow.resolve();assert sha(allow)==sha(source/'bench/differential-divergences.json')
  run(variant+'-metadata',[str(TOOL/'cargo'),'metadata','--offline','--format-version','1','--filter-platform','aarch64-apple-darwin'],harness,60)
  metadata=json.loads((OUT/(variant+'-metadata.stdout')).read_text());packages={p['id']:p for p in metadata['packages']};actualfeatures={packages[n['id']]['name']+'@'+packages[n['id']]['version']:sorted(n['features']) for n in metadata['resolve']['nodes'] if not packages[n['id']]['name'].startswith('issue5183-')};expected={k:v for k,v in expected_features.items() if not k.startswith('issue5183-')};assert actualfeatures==expected
  baseline=pins((source/'Cargo.lock').read_text());resolved=pins((harness/'Cargo.lock').read_text());drift=[v for k,v in resolved.items() if not k[0].startswith('issue5183-') and baseline.get(k)!=v];assert not drift
  save(variant+'-setup-proof.json',{'source_head':HEADS[variant],'module_sha256':sha(module),'allowlist_sha256':sha(allow),'allowlist_resolved_path':str(allow.resolve()),'features':actualfeatures,'dependency_pin_drift':drift,'harness_sha256':sha(harness/'src/main.rs'),'lock_sha256':sha(harness/'Cargo.lock')})
  run(variant+'-build',[str(TOOL/'cargo'),'build','--release','--locked','--offline','-j','1','-vv'],harness,600)
  assert before==source_record(source);binary=OUT/(name+'-binary');shutil.copy2(TARGET/'release'/name,binary);save(variant+'-binary.json',{'path':binary.name,'sha256':sha(binary),'bytes':binary.stat().st_size,'harness_sha256':sha(harness/'src/main.rs')})
  exit_code=run(variant+'-seed',[str(binary)],harness,60,(0,1))
  stdout=(OUT/(variant+'-seed.stdout')).read_text();stderr=(OUT/(variant+'-seed.stderr')).read_text();first=stdout.splitlines()[0]
  assert 'adjudicated classes enabled ["update-oxigraph-integer-lexical-canonicalization"]' in first and str(allow.resolve()) in str(Path(first.split('(',1)[1].split(')',1)[0]).resolve()),first
  assert '4141222487..4141222488' in stdout
  outcome={'variant':variant,'exit':exit_code,'startup':first,'mismatch_line':[s for s in stderr.splitlines() if 'MISMATCH' in s],'step':re.findall(r'^step=(.*)$',stdout,re.M),'source_unchanged':before==source_record(source)};outcomes.append(outcome);save('outcomes.json',outcomes)
  save(variant+'-source-after.json',source_record(source));assert outcome['source_unchanged'];resource();print('OBSERVED',json.dumps(outcome),flush=True)
 status.update(status='complete',outcomes=outcomes)
except BaseException as error:
 status.update(status='stopped',error=repr(error));print('STOP',repr(error),flush=True)
finally:
 status['seconds']=time.monotonic()-START;save('phase-result.json',status);save('resources.json',resources);save('commands.json',commands);print(json.dumps(status),flush=True)
