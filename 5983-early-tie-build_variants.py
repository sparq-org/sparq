"""[GPT-6 Astra] Sequential private builds with one reversible placement control."""
from pathlib import Path
import os,subprocess,json,hashlib,time,shutil
out=Path(__file__).parent;out.joinpath('bins').mkdir(exist_ok=True);root=Path.cwd();source=root/'crates/sparq-engine/src/exec.rs';original=source.read_bytes();assert not subprocess.check_output(['git','status','--porcelain'],text=True)
start=time.time();env=dict(os.environ,CARGO_BUILD_JOBS='2',RAYON_NUM_THREADS='1',CARGO_TARGET_DIR=str(out.parent/'diagnostic/target'));records=[]
old='if scan.to_spo(&rows[first])[2] == scan.to_spo(&rows[edge])[2] {';new='if false && scan.to_spo(&rows[first])[2] == scan.to_spo(&rows[edge])[2] {';assert original.decode().count(old)==1
try:
 for variant,features in [('new-timing',[]),('new-count',['--features','count-alloc']),('early-disabled-count',['--features','count-alloc'])]:
  if variant=='early-disabled-count':
   source.write_text(original.decode().replace(old,new));(out/'early-disabled.diff').write_text(subprocess.check_output(['git','diff','--','crates/sparq-engine/src/exec.rs'],text=True))
  cmd=['/Users/jesght/.cargo/bin/cargo','build','--release','--locked','--offline','--manifest-path','target/bench-wrapper/Cargo.toml']+features;before=time.time();remaining=900-(before-start);assert remaining>0
  with (out/('build-'+variant+'.txt')).open('w') as log:r=subprocess.run(cmd,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=remaining)
  record=dict(variant=variant,command=cmd,exit_code=r.returncode,seconds=time.time()-before,source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),head=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),dirty_control=variant=='early-disabled-count');records.append(record);print(json.dumps(record),flush=True);assert r.returncode==0
  binary=out/'bins'/variant;shutil.copy2(out.parent/'diagnostic/target/release/indexed-topk-diagnostic',binary);record['binary_sha256']=hashlib.sha256(binary.read_bytes()).hexdigest()
  if features:
   c=subprocess.run([str(binary),'selftest'],capture_output=True,text=True,timeout=10);(out/('calibration-'+variant+'.txt')).write_text(c.stdout+c.stderr);assert c.returncode==0
finally:
 source.write_bytes(original)
 (out/'build-results.json').write_text(json.dumps({'records':records,'seconds':time.time()-start,'source_restored':source.read_bytes()==original},indent=2)+'\n')
assert not subprocess.check_output(['git','status','--porcelain'],text=True)
