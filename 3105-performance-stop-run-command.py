import sys,os,subprocess,time,signal,shutil,json
from pathlib import Path
e=Path(__file__).resolve().parent
w=e.parents[1]/'worktrees/issue3105'
name=sys.argv[1];argv=sys.argv[2:]
env=os.environ.copy();env.update(CARGO_BUILD_JOBS='2',CARGO_INCREMENTAL='0',CARGO_NET_OFFLINE='true',CARGO_TARGET_DIR=str(e.parents[1]/'direct-5983/implementation/target'),RAYON_NUM_THREADS='1')
assert shutil.disk_usage(w).free>6*1024**3+64*1024**2
started=time.monotonic();last=started
with (e/(name+'.log')).open('w') as out:
 p=subprocess.Popen(argv,cwd=w,env=env,stdout=out,stderr=subprocess.STDOUT,start_new_session=True)
 stopped=None;minimum=shutil.disk_usage(w).free
 while p.poll() is None:
  free=shutil.disk_usage(w).free;minimum=min(minimum,free)
  if free<6*1024**3+64*1024**2 or time.monotonic()-started>600:
   stopped='disk reserve' if free<6*1024**3+64*1024**2 else '600 second command cap';os.killpg(p.pid,signal.SIGTERM)
   try:p.wait(timeout=5)
   except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
   break
  if time.monotonic()-last>30: print(name,'running',round(time.monotonic()-started),'s free',free,flush=True);last=time.monotonic()
  time.sleep(0.5)
 record=dict(argv=argv,exit=p.returncode,seconds=time.monotonic()-started,minimum_free_bytes=minimum,stopped=stopped,env={k:env[k] for k in ['CARGO_BUILD_JOBS','CARGO_INCREMENTAL','CARGO_NET_OFFLINE','CARGO_TARGET_DIR','RAYON_NUM_THREADS']})
 assert shutil.disk_usage(w).free>6*1024**3
 (e/(name+'.json')).write_text(json.dumps(record,indent=2)+'\n')
 print(json.dumps(record),flush=True)
