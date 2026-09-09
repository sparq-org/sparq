from pathlib import Path
import subprocess, os, json, difflib, re, time
root=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246')
out=Path(__file__).parent
source=root/'crates/sparq-core/src/store.rs'
original=source.read_text()
start=original.index('    fn deleted_count(')
end=original.index('\n    /// The `added` triples',start)
linear="""    fn deleted_count(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> usize {
        let order = perm.order();
        self.deleted.iter().filter(|t| {
            let row = [t[order[0]], t[order[1]], t[order[2]]];
            row >= lo && row <= hi
        }).count()
    }
"""
cases={
 'unconditional_deleted_invalidation':original.replace('if deleted_changed {','if deleted_changed || !ov.deleted.is_empty() {'),
 'remove_deleted_invalidation':original.replace('            ov.invalidate_deleted();','            let _ = deleted_changed;'),
 'ignore_tombstone_insert_flag':original.replace('deleted_changed |= ov.deleted.insert(*t);','ov.deleted.insert(*t);'),
 'ignore_tombstone_remove_flag':original.replace('{ deleted_changed = true; }','{}'),
 'remove_deleted_cache_use':original[:start]+linear+original[end:]
}
env=os.environ.copy();env.update(RAYON_NUM_THREADS='2',CARGO_BUILD_JOBS='2',CARGO_TARGET_DIR='/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
cmd=['/Users/jesght/.cargo/bin/cargo','test','--locked','--offline','-p','sparq-core','--features','overlay-deleted-projections','--lib','store::overlay_deleted_tests::','--','--test-threads=1']
cases['force_cache_in_feature_off'] = original.replace('#[cfg(feature = \"overlay-deleted-projections\")]','').replace('#[cfg(not(feature = \"overlay-deleted-projections\"))]','#[cfg(any())]')
results=[]
try:
 for name,text in cases.items():
  assert text!=original
  source.write_text(text)
  (out/(name+'.diff')).write_text(''.join(difflib.unified_diff(original.splitlines(True),text.splitlines(True),fromfile='store.rs',tofile=name)))
  current_cmd=cmd if name!='force_cache_in_feature_off' else [cmd[0],'test','--locked','--offline','-p','sparq-core','--lib','deleted_projection_feature_off_preserves_main_layout_and_heap','--','--test-threads=1']
  begin=time.monotonic()
  with (out/(name+'.log')).open('w') as log: r=subprocess.run(current_cmd,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=240)
  body=(out/(name+'.log')).read_text()
  summaries=re.findall(r'test result: .*',body)
  assert r.returncode==101 and 'test result: FAILED.' in body, (name,r.returncode)
  results.append(dict(name=name,exit=r.returncode,seconds=time.monotonic()-begin,compiled=True,command=current_cmd,summaries=summaries))
  (out/'results.json').write_text(json.dumps(dict(command=cmd,results=results),indent=2)+'\n')
  print(name,summaries,flush=True)
finally:
 source.write_text(original)
