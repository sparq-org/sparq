# GPT-6 Astra: bounded task-private metadata-only npm; no lifecycle/global install.
from pathlib import Path
import os,sys,time,json,shutil,subprocess,signal
p=Path(__file__).resolve().parent;cfg=json.loads((p/'protocol.json').read_text());w=Path(cfg['worktree'])
def allocated(root):
 total=0
 for q in [root,*root.rglob('*')]:
  try:
   if not q.is_symlink():total+=q.stat().st_blocks*512
  except FileNotFoundError:pass
 return total
def observe():return {'free_bytes':shutil.disk_usage(p).free,'allocated_output_cache_bytes':allocated(p)}
def within(o):return o['free_bytes']>=cfg['minimum_free_bytes'] and o['allocated_output_cache_bytes']<cfg['new_output_cache_limit_bytes'] and time.time()-cfg['start_epoch']<cfg['max_seconds']
label=sys.argv[1];before=observe();assert within(before),before
node=cfg['node'];cli=str(p/'tools/npm-11.17.0/bin/npm-cli.js')
common=['--cache='+str(p/'cache'),'--userconfig='+str(p/'empty-user.npmrc'),'--globalconfig='+str(p/'empty-global.npmrc'),'--registry=https://registry.npmjs.org','--fetch-retries=0','--fetch-timeout=30000','--update-notifier=false','--logs-max=3','--loglevel=verbose']
cmd=[node,cli,*sys.argv[2:],*common]
env={'PATH':str(Path(node).parent)+':/usr/bin:/bin','TMPDIR':str(p/'tmp'),'LANG':'en_US.UTF-8'}
start=time.time();stop=None;low=before['free_bytes'];peak=before['allocated_output_cache_bytes'];proc=None
try:
 with (p/(label+'.log')).open('w') as log:
  proc=subprocess.Popen(cmd,cwd=w,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
  while proc.poll() is None:
   o=observe();low=min(low,o['free_bytes']);peak=max(peak,o['allocated_output_cache_bytes'])
   if not within(o):
    stop='resource/time bound';os.killpg(proc.pid,signal.SIGTERM);proc.wait(timeout=10);break
   time.sleep(.5)
except BaseException:
 if proc is not None and proc.poll() is None:
  os.killpg(proc.pid,signal.SIGTERM);proc.wait(timeout=10)
 raise
finally:
 after=observe()
 r={'command':cmd,'cwd':str(w),'environment':env,'start_epoch':start,'seconds':time.time()-start,'exit_code':proc.returncode if proc else None,'stop':stop,'before':before,'after':after,'minimum_observed_free_bytes':min(low,after['free_bytes']),'maximum_observed_allocated_bytes':max(peak,after['allocated_output_cache_bytes'])}
 (p/(label+'.json')).write_text(json.dumps(r,indent=2)+'\n')
 print(json.dumps(r));print('\n'.join((p/(label+'.log')).read_text().splitlines()[-16:]))
sys.exit(0 if proc.returncode==0 and stop is None else 1)
