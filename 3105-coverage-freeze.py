from pathlib import Path
import json,hashlib,subprocess,re,shutil,datetime
E=Path(__file__).resolve().parent;W=E.parents[1]/'worktrees/issue3105';S=W/'crates/sparq-engine/src/exec.rs';BASE='6e86f1d0ba447aa78a50800337a7a379702fa999';MAIN='e53464c73f31f7aca800f3867ac054c36408e346'
assert shutil.disk_usage(E).free>6509559808
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
git=lambda *args:subprocess.check_output(['git',*args],cwd=W,text=True)
head=git('rev-parse','HEAD').strip();assert head=='19763bfab1dce196a654c899b172e7b24d70bc59';assert not git('status','--porcelain=v1')
source=S.read_text();assert source==(E/'final-source.rs').read_text()
old=git('show',BASE+':crates/sparq-engine/src/exec.rs');needle='    pub(super) fn observe(index: usize) {'
assert source[source.index(needle,source.index('mod capped_rhs_tests {')):]==old[old.index(needle,old.index('mod capped_rhs_tests {')):]
(E/'commit.txt').write_text(git('show','-s','--format=fuller',head));(E/'delta.diff').write_text(git('diff',BASE,head,'--','crates/sparq-engine/src/exec.rs'));(E/'full.diff').write_text(git('diff',MAIN,head,'--','crates/sparq-engine/src/exec.rs'))
(E/'toolchain.txt').write_text(subprocess.check_output(['rustc','-Vv'],cwd=W,text=True))
contexts={
 'context-driver-helper.rs':source[source.index('fn eval_bgp_binary_capped('):source.index('/// Distinct (non-repeated) variable positions')],
 'context-tests.rs':source[source.index('// [GPT-6 Astra] Actual public-query path and physical RHS-work witness'):],
 'context-conjunctive.rs':source[source.index('pub(crate) fn is_conjunctive('):source.index('fn split_sargable(')],
}
for n,s in contexts.items():(E/n).write_text(s)
core=(W/'crates/sparq-core/src/store.rs').read_text().splitlines();(E/'context-built.rs').write_text('\n'.join(core[:40])+'\n')
engine=(W/'crates/sparq-engine/Cargo.toml').read_text().splitlines();(E/'context-features.toml').write_text('\n'.join(engine[24:37]+engine[474:485])+'\n')
receipts=[]
for p in sorted(E.glob('*.json')):
 try:r=json.loads(p.read_text())
 except (json.JSONDecodeError,UnicodeDecodeError):continue
 if isinstance(r,dict) and 'argv' in r:receipts.append({'file':p.name,**r})
(E/'commands.json').write_text(json.dumps(receipts,indent=2)+'\n')
(E/'binary').mkdir(exist_ok=True);bins={}
for name in ['restored-default','pinned-compact','pinned-no-default','pinned-semijoin']:
 log=(E/(name+'.log')).read_text();assert 'test result: ok. 7 passed; 0 failed' in log
 p=Path(re.search(r'Running unittests src/lib.rs \(([^)]+)\)',log).group(1));dest=E/'binary'/name;shutil.copy2(p,dest);bins[name]={'sha256':sha(dest),'bytes':dest.stat().st_size,'source_sha256':sha(S),'command':json.loads((E/(name+'.json')).read_text())}
(E/'binary-provenance.json').write_text(json.dumps(bins,indent=2)+'\n')
assert 'test result: ok. 21 passed; 0 failed; 1 ignored' in (E/'ask-early-exit.log').read_text()
assert json.loads((E/'pinned-clippy.json').read_text())['exit']==0
assert 'mapfile: command not found' in (E/'pinned-preflight.log').read_text()
controls=json.loads((E/'controls.json').read_text());assert len(controls)==2 and all(c['killed_by_executed_assertion'] for c in controls)
prior=[]
for p in [E.parent/'manifest.json',E.parent/'performance/manifest.json',E.parent/'performance-completion/manifest.json']:
 m=json.loads(p.read_text())
 for name,v in m['files'].items():
  f=p.parent/name;assert f.stat().st_size==v['bytes'] and sha(f)==v['sha256'],name
 prior.append({'path':str(p),'sha256':sha(p),'verified_files':len(m['files'])})
(E/'prior-verified.json').write_text(json.dumps(prior,indent=2)+'\n')
report={
 'head':head,'parent':BASE,'main_base':MAIN,'author':'Actual GPT-6 Astra xhigh; existing historical attribution preserved; ordinary unsigned local commit, no persistent config or hook changes.',
 'scope':'One file: crates/sparq-engine/src/exec.rs, +260/-2 from reviewed6e86. One production statement clears stale slot before scan; other changes are cfg(test) instrumentation/tests. Existing test suffix byte-identical; no API/dependency/storage/cache-policy/planner/hash-build change.',
 'results':{'focused_default':'7 passed','focused_no_engine_defaults':'7 passed; core dev dependencies still enable parallel/mmap/dict-spill, not a lean wasm run','focused_compact_index':'7 passed; explicit three-permutation hash/order/reuse expectations','focused_semijoin_bitmap':'7 passed','ask_early_exit':'21 passed, 1 existing ignored timing probe','clippy':'cargo clippy --locked --offline -p sparq-engine --lib --tests -- -D warnings: PASS','diff_check':'PASS','preflight':'FAIL solely because Bash3 lacks mapfile in privacy-claims script; no workaround/weakening. Linux preflight required.'},
 'actual_paths':{
  'six_permutations':'70000-row public SELECT LIMIT70001, projecting35000 integer values twice each; full query bag AND generated exact bag agree. q pattern1: block0 bind, block1024 merge(requestSome0, scanned), block65536 bind. r pattern2: hash(None,scan), merge(Some0,scan), hash(None,scan); actual RHS order s throughout.',
  'three_permutations':'Same complete oracle and multiplicity pass. q bind/hash/bind; r requestsNone in allthree blocks, actual orderx, scans once then reuses twice. Requested and actual order are not conflated. Explicit branch differs because the built index set lacks PSO.',
  'disconnected':'Public capped query executes one cross_product_ref RHS step, produces six rows and exact two-value bag with multiplicitythree; full query agrees. Small one-block cross, not a multi-block memory stress.',
  'named_view_overlay':'Eligible named subquery reaches RHS scan; visible view with empty default retains same bag; hidden view returns zero rows and no RHS steps. Tombstone removes one q row from a fork and eligible count decreases3→2. Correlated EXISTS variant returns projected multiplicity2 then1 after deletion.',
  'exists_boundary':'Initial desired EXISTS engagement assertion failed: is_conjunctive/filter_scope_ok deliberately reject EXISTS. Final test pins zero RHS-cache steps for this fallback and separately exercises eligible named/view/overlay queries. No guard broadened or forced eligibility.'},
 'lifetime':'Rust assignment evaluates replacement RHS before dropping old LHS; old Some((sort,Bindings)) therefore retained rows through scan(). New *slot=None drops old ownership before calling scan. Actual-helper catch_unwind sentinel proves slot empty after replacement scan aborts; normal replacement still yields expected row. This is observable ownership-state proof, not allocator/time measurement, and catches unwind only.',
 'controls':controls,
 'controls_limits':'Requested-sort mutant returns the same full bag on this fixture, then fails actual scanned/requested-path assertion. This is an invalidation/work control, not a demonstrated semantic failure. Lifetime mutant keeps actual old slot populated after sentinel panic. Expected caught panic output is not an unexpected test failure. Initial and final control executions retained; final controls use exact final source plus documented mutation.',
 'iterations':'First two compile attempts corrected test import placement and query_view arity; third execution exposed genuine existing EXISTS fallback, not production failure. Initial compact run preserved full bag but correctly differed from assumed six-index path; final index-set-specific assertions document actual three-index behavior. All initial logs/source snapshots retained. Old test formatting restored to keep prior source byte-identical.',
 'measurements':'No new timings or allocation matrix. Prior exact6e86 positive local performance screen and all frozen binaries/artifacts remain unchanged and verified. Do not attribute those timings to this new head or claim a measured peak reduction from slot clear.',
 'limits':['No full workspace clippy/tests, wasm build/byte equality, conformance or canonical perf ratchet run here; authoritative CI required before admission.','No midquery asynchronous cancellation injection; existing cancellation/deadline/rowbudget tests and armed-budget nonreuse checks passed unchanged.','Sum of retained per-pattern RHS remains a resource risk outside the measured two-RHS fixture. Releasing stale per-slot data only avoids replacement overlap; it is not a retention cap or budget policy change.','Nested-public-query budget issue6476 remains untouched. No production query policy or guard changes beyond stale slot lifetime.','No blanket all-features claim: tested default, no engine defaults, core compact-index and semijoin-bitmap configurations only.','No remote action, model call, registry/release/EC2 operation or further lane started.'],
 'decision':'Ready for actual focused Opus review and subsequent full validation; no merge, performance-admission or issue-closure claim.',
 'resource':{'started_utc':'2026-09-10T01:36:22Z','completed_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'free_bytes':shutil.disk_usage(E).free,'min_observed_command_free_bytes':min(r['minimum_free_bytes'] for r in receipts if 'minimum_free_bytes' in r),'floor_bytes':6509559808,'jobs':2,'incremental':False,'offline_locked':True,'commands_pending':False},
 'prior_verified':prior,
}
(E/'report.json').write_text(json.dumps(report,indent=2)+'\n')
packet='# Issue3105 focused coverage and stale-slot lifetime delta\n\nActual GPT-6 Astra xhigh. This extends the reviewed6e86 candidate; prior source review approved validation, not admission. Full phase1/performance bundles remain unchanged.\n\n'+json.dumps(report,indent=2)+'\n\n## Exact delta from6e86\n\n```diff\n'+(E/'delta.diff').read_text()+'```\n'
for name in ['context-driver-helper.rs','context-tests.rs','context-built.rs','context-features.toml']:
 packet+='\n## '+name+'\n\n```rust\n'+(E/name).read_text()+'```\n'
# Only the exact scope classifier through filter_scope_ok, rather than unrelated comparison helpers.
x=contexts['context-conjunctive.rs'];cut=x.find('fn extract_sargable(');x=x[:cut] if cut>=0 else x
packet+='\n## Existing conjunctive/EXISTS scope boundary\n\n```rust\n'+x+'```\n'
for name in ['restored-default','pinned-compact','pinned-no-default','pinned-semijoin','ignore-requested-sort','retain-old-through-scan','ask-early-exit','pinned-clippy','pinned-preflight']:
 packet+='\n## Executed '+name+'\n\n```text\n'+(E/(name+'.log')).read_text().replace(str(E.parents[1]),'<task-root>')+'```\n'
packet+='\n## Evidence context\n\nFull change against main is in full.diff; complete final source is final-source.rs. Exact command argv/environment/reserve records are commands.json and individual receipts. Four executed candidate test binaries are frozen with hashes in binary-provenance.json; control binaries are identified by actual compiled hashes in controls.json, with complete mutated sources and diffs retained. Original compile/fixture failures and first control pair are preserved. Unchanged scan/filter/join/budget/view consumer context remains in the prior phase1 packet and admission/source-validity-context.txt; this focused packet omits duplicated unchanged storage/view bodies. Core dev feature unification is shown above. No external independent review was performed by this author.\n'
(E/'review-packet.md').write_text(packet)
files={str(p.relative_to(E)):{'sha256':sha(p),'bytes':p.stat().st_size} for p in sorted(E.rglob('*')) if p.is_file() and p.name!='manifest.json'}
(E/'manifest.json').write_text(json.dumps({'head':head,'files':files,'count':len(files),'bytes':sum(x['bytes'] for x in files.values())},indent=2)+'\n')
print(json.dumps({'head':head,'files':len(files),'bytes':sum(x['bytes'] for x in files.values()),'manifest_sha256':sha(E/'manifest.json'),'packet_bytes':(E/'review-packet.md').stat().st_size,'packet_sha256':sha(E/'review-packet.md'),'free_bytes':shutil.disk_usage(E).free},indent=2))
