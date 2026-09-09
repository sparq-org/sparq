# [GPT-6 Astra] Bounded behavioral controls; restore exact source after each.
import os, pathlib, subprocess, json, hashlib
root = pathlib.Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246')
out = pathlib.Path(__file__).parent
source = root / 'crates/sparq-core/src/store.rs'
original = source.read_text()
env = dict(os.environ, RAYON_NUM_THREADS='2', CARGO_BUILD_JOBS='2', CARGO_TARGET_DIR='/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0')
command = ['/Users/jesght/.cargo/bin/cargo','test','--locked','--offline','-p','sparq-core','--lib','store::overlay_deleted_tests::','--','--test-threads=1']
mutants = {
 'remove_cache_use': ('let del = self.deleted_count(perm, lo, hi);', 'let order = perm.order(); let del = self.deleted.iter().filter(|t| { let r = [t[order[0]], t[order[1]], t[order[2]]]; r >= lo && r <= hi }).count();'),
 'remove_deleted_invalidation': ('self.added_by_perm.iter_mut().chain(&mut self.deleted_by_perm)', 'self.added_by_perm.iter_mut()'),
 'exclude_upper_bound': ('rows.partition_point(|r| *r <= hi) - rows.partition_point(|r| *r < lo)', 'rows.partition_point(|r| *r < hi) - rows.partition_point(|r| *r < lo)'),
 'omit_deleted_heap_accounting': ('.chain(&self.deleted_by_perm)', ''),
}
results=[]
try:
 for name,(before,after) in mutants.items():
  assert original.count(before)==1,(name, original.count(before))
  mutated=original.replace(before,after)
  source.write_text(mutated)
  (out/(name+'.diff')).write_text(subprocess.check_output(['git','diff','--','crates/sparq-core/src/store.rs'],cwd=root,text=True))
  with (out/(name+'.log')).open('w') as log:
   r=subprocess.run(command,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=240)
  text=(out/(name+'.log')).read_text()
  result=dict(name=name,exit=r.returncode,killed=r.returncode==101 and 'test result: FAILED' in text and 'could not compile' not in text,source_sha256=hashlib.sha256(mutated.encode()).hexdigest())
  results.append(result)
  print(json.dumps(result),flush=True)
  source.write_text(original)
finally:
 source.write_text(original)
 (out/'results.json').write_text(json.dumps(results,indent=2)+'\n')
assert len(results)==4 and all(x['killed'] for x in results)
