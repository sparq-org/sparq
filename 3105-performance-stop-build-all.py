from pathlib import Path
import subprocess,sys,json,hashlib,shutil,time
e=Path(__file__).resolve().parent;t=e.parents[1]/'direct-5983/implementation/target';(e/'binary').mkdir(exist_ok=True);r=[];start=time.monotonic()
for mode in ['timing','count']:
 for source in ['baseline','candidate']:
  assert shutil.disk_usage(e).free>6*1024**3+64*1024**2
  name='final-build-'+source+'-'+mode
  argv=['cargo','build','--release','--locked','--offline','--manifest-path',str(e/source/'Cargo.toml')]
  if mode=='count':argv+=['--features','count-alloc']
  subprocess.run([sys.executable,str(e/'run-command.py'),name,*argv],check=True)
  receipt=json.loads((e/(name+'.json')).read_text());r.append(receipt)
  if receipt['exit']!=0:break
  binary=t/'release/capped-rhs-diagnostic';assert shutil.disk_usage(e).free>6*1024**3+64*1024**2+binary.stat().st_size
  dest=e/'binary'/(source+'-'+mode);shutil.copy2(binary,dest)
  r[-1]['binary_sha256']=hashlib.sha256(dest.read_bytes()).hexdigest();r[-1]['binary_bytes']=dest.stat().st_size
 else:continue
 break
(e/'builds.json').write_text(json.dumps(r,indent=2)+'\n')
