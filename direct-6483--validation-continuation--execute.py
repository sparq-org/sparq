from pathlib import Path
import os, errno, subprocess, time, signal, json, hashlib, shutil, sys, stat as statmod
p=Path(__file__).resolve().parent
r=p.parent.parent
w=r/'worktrees/issue6483'
old=p.parent/'implementation'
started=time.monotonic()
commands=[]; resources=[]; disappeared=0
sha=lambda b:hashlib.sha256(b).hexdigest()
def put(name,value):
 (p/name).write_text(json.dumps(value,indent=2)+'\n' if not isinstance(value,str) else value)
def enoent_only(exc):
 global disappeared
 if exc.errno != errno.ENOENT: raise exc
 disappeared+=1
def allocated(root):
 total=0
 for parent,dirs,files in os.walk(root,onerror=enoent_only,followlinks=False):
  for name in files:
   path=Path(parent)/name
   try:
    stat=path.lstat()
    if not statmod.S_ISLNK(stat.st_mode): total+=stat.st_blocks*512
   except OSError as exc: enoent_only(exc)
 return total
def check(initial=False):
 amounts={str(root.relative_to(r)):allocated(root) for root in (w,old,p)}
 row={'elapsed_seconds':time.monotonic()-started,'allocations':amounts,'allocated_bytes':sum(amounts.values()),'free_bytes':shutil.disk_usage(p).free,'enoent_entries':disappeared}
 resources.append(row);put('resources.json',resources)
 assert row['free_bytes'] >= (2214592512 if initial else 2147483648),'free floor'
 assert row['allocated_bytes'] < 201326592-8388608,'combined cap less export reserve'
 assert row['elapsed_seconds'] < 180,'180s phase cap'
 return row
def run(name,argv,limit,expected=0):
 check(); start=time.monotonic(); stopped=None
 with (p/(name+'.stdout')).open('wb') as out,(p/(name+'.stderr')).open('wb') as err:
  proc=subprocess.Popen(argv,cwd=w,env=env,stdout=out,stderr=err,start_new_session=True)
  while proc.poll() is None:
   try:
    check(); assert time.monotonic()-start<limit,'per-command cap'
   except Exception as exc:
    stopped=str(exc);os.killpg(proc.pid,signal.SIGTERM)
    try:proc.wait(timeout=2)
    except subprocess.TimeoutExpired:os.killpg(proc.pid,signal.SIGKILL);proc.wait()
    break
   time.sleep(.2)
  code=proc.wait()
 row={'name':name,'argv':argv,'exit':code,'expected_exit':expected,'seconds':time.monotonic()-start,'stopped':stopped}
 commands.append(row);put('commands.json',commands);print(json.dumps(row),flush=True)
 check();assert stopped is None and code==expected,'unexpected command result; stop'
try:
 initial=check(True);put('initial-resource.json',initial)
 manifest=json.loads((old/'manifest.json').read_text())
 for item in manifest['files']:assert sha((old/item['path']).read_bytes())==item['sha256'],item['path']
 assert sha((old/'manifest.json').read_bytes())=='dfa1ff9fa266fe27756028bd148a7fccb77898bf8926c9c52bcd4bd5aaef6157'
 provenance=json.loads((old/'provenance.json').read_text())
 for item in [provenance['compiler']]+provenance['externs']:assert sha(Path(item['path']).read_bytes())==item['sha256'],item['path']
 source=w/'crates/sparq-bench/src/update_fuzz.rs'
 assert sha(source.read_bytes())==provenance['source_sha256']
 assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=w,text=True).strip()=='f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464'
 assert subprocess.check_output(['git','branch','--show-current'],cwd=w,text=True).strip()=='codex/update-comparator-raw-duplicates'
 cb=old/'candidate/out/issue6483_candidate-fixed'
 assert sha(cb.read_bytes())=='2e4a928326bc4df691700e21e4735a5c34acf297b3f5e624c8e298463fff921f'
 assert sha((old/'harness/bench/differential-divergences.json').read_bytes())==provenance['allowlist_sha256']
 (p/'tmp').mkdir();(p/'control/out').mkdir(parents=True);(p/'clippy-out').mkdir()
 control=p/'control/update_fuzz.rs';control.write_bytes((old/'control/update_fuzz.rs').read_bytes())
 driver=p/'control/driver.rs';driver.write_text((old/'control/driver.rs').read_text().replace(str(old/'control/update_fuzz.rs'),str(control)))
 argv=json.loads((old/'control-compiler-argv.json').read_text())
 argv=[arg.replace(str(old/'control/driver.rs'),str(driver)).replace(str(old/'control/out'),str(p/'control/out')) for arg in argv]
 controlled=provenance['environment'].copy();controlled['TMPDIR']=str(p/'tmp')
 env={k:v for k,v in os.environ.items() if not k.startswith(('SPARQ_','CARGO_PROFILE_')) and k not in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS')};env.update(controlled)
 put('provenance.json',dict(provenance,environment=controlled,old_manifest_verified=40,candidate_binary_sha256=sha(cb.read_bytes()),candidate_rebuilds=0,control_source_sha256=sha(control.read_bytes())))
 put('protocol.json',{'candidate':'existing exact binary, one full26-test run','control':'one removed-guard module build and one filtered regression, expected101 and Same assertion','clippy':'same candidate module, metadata-only -D warnings','limits_seconds':180,'accounted_roots':[str(x.relative_to(r)) for x in (w,old,p)],'max_combined_bytes':201326592,'export_reserve':8388608,'controller_change':'Only ENOENT from allocation traversal/stat is tolerated; removed temporary LOAD files no longer abort unrelated test. All other errors/time/free/growth guards retained.','before_execution':True})
 run('candidate-tests',[str(cb),'--test-threads=1','--nocapture'],65)
 assert '26 passed; 0 failed' in (p/'candidate-tests.stdout').read_text(),'full26 missing'
 run('control-build',argv,60)
 mb=p/'control/out/issue6483_control-fixed'
 run('control-test',[str(mb),'--exact','update_fuzz::tests::raw_duplicate_redistribution_is_rejected','--test-threads=1','--nocapture'],20,101)
 assert 'equal-total raw duplicate redistribution returned Same' in (p/'control-test.stderr').read_text(),'missing behavioral failure'
 clippy=Path(provenance['compiler']['path']).with_name('clippy-driver');assert clippy.is_file(),'warm Clippy missing'
 ca=json.loads((old/'candidate-compiler-argv.json').read_text());ca[0]=str(clippy)
 ca[ca.index('--out-dir')+1]=str(p/'clippy-out');ca[ca.index('--emit=dep-info,link')]='--emit=metadata';ca+=['-D','warnings']
 run('clippy-tests',ca,40)
 put('binary-provenance.json',[{'variant':n,'path':str(b),'sha256':sha(b.read_bytes()),'bytes':b.stat().st_size} for n,b in [('candidate',cb),('control',mb)]])
 for item in manifest['files']:assert sha((old/item['path']).read_bytes())==item['sha256'],'old frozen changed: '+item['path']
 assert sha(source.read_bytes())==provenance['source_sha256']
 put('phase-result.json',{'status':'complete','seconds':time.monotonic()-started,'commands_terminal':len(commands),'pending':False,'old_manifest_verified_after':40,'source_unchanged':True})
except Exception as exc:
 put('phase-result.json',{'status':'stopped','reason':str(exc),'seconds':time.monotonic()-started,'commands_terminal':len(commands),'pending':False});print('STOP',str(exc),flush=True);sys.exit(1)
