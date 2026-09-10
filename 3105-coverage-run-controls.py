from pathlib import Path
import subprocess,sys,json,shutil,difflib,re,hashlib
E=Path(__file__).resolve().parent; W=E.parents[1]/'worktrees/issue3105'; P=W/'crates/sparq-engine/src/exec.rs'; source=P.read_text(); outcomes=[]
(E/'final-source.rs').write_text(source)
mutants=[('ignore-requested-sort','*cached_sort != sort','{ let _ = (cached_sort, sort); false }','capped_rhs_changing_sort_mixed_kernels_preserves_full_bag'),('retain-old-through-scan','        *slot = None;\n        *slot = Some((sort, scan()));','        *slot = Some((sort, scan()));','capped_rhs_replacement_releases_old_slot_before_scan')]
try:
 for name,old,new,test in mutants:
  assert shutil.disk_usage(W).free>6509559808
  assert source.count(old)==1
  changed=source.replace(old,new,1);P.write_text(changed)
  (E/(name+'-source.rs')).write_text(changed)
  (E/(name+'.diff')).write_text(''.join(difflib.unified_diff(source.splitlines(True),changed.splitlines(True),fromfile='candidate/exec.rs',tofile=name+'/exec.rs')))
  subprocess.run([sys.executable,str(E/'run-command.py'),name,'cargo','test','--locked','--offline','-p','sparq-engine','--lib',test,'--','--nocapture','--test-threads=1'],check=True)
  r=json.loads((E/(name+'.json')).read_text());log=(E/(name+'.log')).read_text()
  killed=r['exit']==101 and 'test result: FAILED. 0 passed; 1 failed' in log and 'could not compile' not in log
  binary=Path(re.search(r'Running unittests src/lib.rs \(([^)]+)\)',log).group(1))
  outcomes.append({'name':name,'test':test,'killed_by_executed_assertion':killed,'exit':r['exit'],'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'binary_bytes':binary.stat().st_size,'interpretation':'Path/invalidation assertion after all full-bag assertions passed, not a demonstrated wrong result.' if name.startswith('ignore') else 'Actual slot remains populated after replacement scan panic; candidate empties it before calling scan.'})
  assert killed,(name,log)
finally:
 P.write_text(source);(E/'controls.json').write_text(json.dumps(outcomes,indent=2)+'\n')
print(json.dumps(outcomes))
