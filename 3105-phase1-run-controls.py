from pathlib import Path
import subprocess,sys,json,shutil,difflib,re
e=Path(__file__).resolve().parent;w=e.parent/'worktrees/issue3105';p=w/'crates/sparq-engine/src/exec.rs';source=p.read_text();out=[]
mutants=[('invalidate-per-block','    while start < seed_all.rows.len() {','    while start < seed_all.rows.len() {\n        for slot in &mut rhs_cache { *slot = None; }'),('ignore-requested-sort','*cached_sort != sort','false'),('ignore-budget','let reuse_rhs = !budget::active();','let reuse_rhs = true;')]
try:
 for name,old,new in mutants:
  assert shutil.disk_usage(w).free>6*1024**3+80*1024**2
  assert source.count(old)==1,(name,source.count(old));changed=source.replace(old,new,1);p.write_text(changed)
  (e/(name+'.diff')).write_text(''.join(difflib.unified_diff(source.splitlines(True),changed.splitlines(True),fromfile='candidate/exec.rs',tofile=name+'/exec.rs')))
  subprocess.run([sys.executable,str(e/'run-command.py'),name,'cargo','test','--locked','--offline','-p','sparq-engine','--lib','capped_rhs_','--','--nocapture','--test-threads=1'],check=True)
  result=json.loads((e/(name+'.json')).read_text());log=(e/(name+'.log')).read_text()
  valid=result['exit']==101 and 'test result: FAILED' in log and 'could not compile' not in log
  out.append({'name':name,'killed_by_executed_assertion':valid,'exit':result['exit']})
  binary=Path(re.search(r'Running unittests src/lib.rs \(([^)]+)\)',log).group(1));shutil.copy2(binary,e/'binary'/name)
  if not valid: raise RuntimeError('uncalibrated control: '+name)
finally:
 assert shutil.disk_usage(w).free>6*1024**3+len(source)
 p.write_text(source);(e/'controls.json').write_text(json.dumps(out,indent=2)+'\n')
