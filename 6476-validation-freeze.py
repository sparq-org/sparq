from pathlib import Path
import subprocess,json,hashlib,re,shutil,time
v=Path(__file__).resolve().parent;wt=Path.cwd();base='d41ec9fcb796504d85f5a5247a5fdc9eb3e65de5';head=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip();assert subprocess.check_output(['git','status','--porcelain'],text=True)==''
files=subprocess.check_output(['git','diff','--name-only',base,head],text=True).splitlines();(v/'source').mkdir(exist_ok=True)
for f in files:
 p=v/'source'/f;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(Path(f).read_bytes())
diff=subprocess.check_output(['git','diff','--no-ext-diff',base,head],text=True);(v/'full.diff').write_text(diff)
cache=Path('/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/release/deps');final=v/'final-rlibs';final.mkdir(exist_ok=True)
for name in ['libsparq_engine-d082493c198a3f5b.rlib','libsparq_engine-d082493c198a3f5b.rmeta','sparq_engine-d082493c198a3f5b.d','libsparq_core-127354fe878a2f23.rlib','libsparq_core-127354fe878a2f23.rmeta']:shutil.copy2(cache/name,final/name)
receipts=[]
for p in sorted(v.glob('*.json')):
 d=json.loads(p.read_text())
 if isinstance(d,dict) and 'command' in d and 'exit_code' in d:receipts.append(d)
(v/'command-index.json').write_text(json.dumps(receipts,indent=2)+'\n')
controls=[]
for label in ['final-omit-limits','final-omit-sticky','final-omit-bytes']:
 log=(v/('test-'+label+'.log')).read_text();m=re.search(r'test result: FAILED\. (\d+) passed; (\d+) failed;',log);assert m
 controls.append({'control':label,'compiled':True,'passed':int(m[1]),'failed':int(m[2]),'failures':re.findall(r'^    (exec::budget::nested_budget_tests::\w+)$',log,re.M)})
assert (v/'compile-final-sealed.log').read_text().count('error[E0603]')==4
report={'issue':6476,'head':head,'base':base,'source_status':'clean committed candidate for independent review, not merge approval','implementation_model':'GPT-6 Astra xhigh','source_files':{f:hashlib.sha256(Path(f).read_bytes()).hexdigest() for f in files},'diff_sha256':hashlib.sha256(diff.encode()).hexdigest(),'diffstat':{'files':7,'insertions':519,'deletions':208},'implementation':['Private Guard/install plus crate-visible synchronous with_budget FnOnce scope; full prior Limits and EXCEEDED restored on return/unwind','All14 production caller bodies mechanically preserved inside same scope; view guards outside, trace guard inside; nine prior test install scopes migrated','Limits/snapshot/hit/why/fanout restricted to exec and documented four synchronous production consumers; QueryBudget nested behavior documented'],
'validation':{'fresh_public_final':{'cases':['no nested call','unlimited nested ASK','explicit-budget nested ASK','nested parse error'],'cancellation_enforced':[True,True,True,True],'baseline_enforced':[True,False,False,True],'each_callback_calls':1,'same_installing_thread':True,'subsequent_top_level_ASK':True},'actual_module':{'passed':15,'failed':0,'test_source':'exact complete copied budget module, including explicit caught top-level unwind cleanup','boundary':'Private copied TLS differs from linked production-library TLS; state tests exercise actual copied code, public callback tests call fresh production library. Not full engine Cargo test suite.'},'mutants':controls,'internal_sibling_sealing':{'sealed_E0603_errors':4,'opened_control_compile_exit':0,'runtime_executed':False},'production_parallel':{'path':'scan JSON par_chunks before snapshot hit','two_distinct_Rayon_workers':True,'fixture_rows':60000,'complete_rows_and_materialized_oracle_count':True,'budget':'Cancellation owner installed, flag false; raw harness field armed_cancel=false denotes flag value, not absence of installed budget.','source':'Task-private exact engine copy plus one observer call/private helper; production has no observer. First observer call per worker rendezvous establishes two real workers; watchdog30sec; no timing/throughput claim.'},'features':{'cargo_default':'compiled; public four-case witness passed at final source','cargo_feature_bundle':['result-cache','explain-json','params','service-local','vectorized','cs-planner'],'feature_bundle_result':'compiled and public four-case witness passed before final nine-line cfg(test)-only top-unwind assertion','no_parallel':'Actual engine compiled with no feature cfgs against existing pinned dependency artifacts, and public four-case witness passed. Core dependency remains parallel-enabled; not a full serial/wasm dependency-graph gate. Executed before cfg(test)-only final assertion.','clippy':'Direct installed clippy-driver -D warnings passed whole production library under affected-feature bundle and exact final copied budget tests. Cargo dependency-package clippy rejected feature selection, so no ordinary full Cargo clippy claim.'}},
'failures_and_limits':['Initial shipping-profile build stopped on prior disk floor before engine compilation; preserved in frozen implementation bundle.','Serial Cargo --locked rejected changed feature lock; offline regenerated scratch lock changed package versions, rejected by subset assertion and never built. No package installed or production dependency changed; direct pinned-artifact engine feature-off check used instead.','Preflight exit1: Bash3 privacy script mapfile unavailable; other mechanical checks passed.','No full workspace tests/clippy, remote SERVICE build/runtime, wasm, Miri, or full ratchets run locally. Those remain required supported CI obligations.','Parallel snapshot witness covers one of four production fanouts dynamically; all four synchronous lifetimes audited in source. Raw Copy snapshot remains a trusted exec-internal boundary, not a type-level lifetime proof against arbitrary future escapes.'],
'ownership_and_service':['Private scoped installer prevents caller-forgotten/out-of-order guards; borrowed budget and !Send marker retained. Each saved cancellation owner is borrowed by a live enclosing with_budget frame.','Snapshots synchronously join/collect before parent evaluation returns. No production snapshot consumer installs parent TLS on workers or returns raw Limits.','All three SERVICE byte savepoint paths restore only their parent transaction extra_bytes/sticky error. A synchronous nested with_budget may complete in a handler before parent interning/rollback; its full parent restoration makes this safe. A savepoint must not be applied while another frame is active. Existing code does not do so.'],
'resources':{'jobs':1,'incremental':False,'offline':True,'profile':'O3/unwind/no-LTO/codegen16 local correctness only','growth_limit':536870912,'free_floor':2147483648,'max_measured_growth':max(x['max_growth_bytes'] for x in receipts),'min_measured_free':min(x['min_free_bytes'] for x in receipts),'free_at_freeze':shutil.disk_usage('/private/tmp').free},'no_remote_mutations':True,'no_admission_claim':True}
(v/'report.json').write_text(json.dumps(report,indent=2)+'\n')
# Public review packet: repository source, relative paths, structured observations only.
packet='# Nested query budget restoration — issue6476\n\n'+json.dumps(report,indent=2)+'\n\n## Exact final diff\n```diff\n'+diff+'\n```\n'
s=Path('crates/sparq-engine/src/exec.rs').read_text();a=s.index('pub(crate) mod budget {');b=s.index('\n}\n',a)+2
packet+='\n## Complete budget module — crates/sparq-engine/src/exec.rs\n```rust\n'+s[a:b]+'\n```\n'
caller_names={'lib.rs':['query_prepared_with_budget','ask_prepared_with_budget','query_json_prepared_with_budget','query_json_chunks_with_budget','query_json_stream_prepared_with_budget','count_prepared_with_budget'],'cache.rs':['eval'],'construct.rs':['construct_prepared_with_budget','describe_prepared_with_budget','construct_or_describe_with_budget'],'explain.rs':['explain_analyze_with_budget'],'explain_json.rs':['explain_plan_analyze_with_budget'],'update.rs':['update_in_place_prepared_with_budget','update_in_place_core']}
def function_text(text,name):
 m=re.search(r'^(?:pub(?:\([^)]*\))?\s+)?fn '+name+r'\b',text,re.M);assert m,name;a=m.start();b=text.index('\n}\n',a)+2;return text[a:b]
for f,names in caller_names.items():
 t=Path('crates/sparq-engine/src/'+f).read_text()
 for name in names:packet+='\n## Caller — crates/sparq-engine/src/'+f+'::'+name+'\n```rust\n'+function_text(t,name)+'\n```\n'
# Complete enclosing branch around each snapshot, plus signature/owning scope source provenance.
lines=s.splitlines()
for needle in ['if scan_rows.len() >= PAR_THRESHOLD {','if bindings.rows.len() >= PAR_THRESHOLD {','let snap = EngineSnapshot(budget::snapshot());','let limits = budget::snapshot();']:
 hits=[i for i,x in enumerate(lines) if needle in x]
 if needle=='let limits = budget::snapshot();':hits=[i for i in hits if i>9000]
 for i in hits:
  if i<2400:continue
  if needle.startswith('if '):start=i
  else:
   start=i
   while start>0 and not re.match(r'    +if .*\{',lines[start]):start-=1
  indent=len(lines[start])-len(lines[start].lstrip());end=start+1
  while end<len(lines) and lines[end] != ' '*indent+'}':end+=1
  packet+='\n## Snapshot branch — crates/sparq-engine/src/exec.rs:'+str(start+1)+'\n```rust\n'+'\n'.join(lines[max(0,start-5):min(len(lines),end+6)])+'\n```\n'
# Complete actual SERVICE functions owning all three savepoints.
fn_matches=list(re.finditer(r'^(?:pub(?:\([^)]*\))?\s+)?fn (\w+)\b',s,re.M));names=[]
for m in re.finditer(r'let byte_mark = budget::byte_savepoint\(\);',s):
 prev=[f for f in fn_matches if f.start()<m.start()][-1].group(1)
 if prev not in names:names.append(prev)
for name in names:packet+='\n## SERVICE owner — crates/sparq-engine/src/exec.rs::'+name+'\n```rust\n'+function_text(s,name)+'\n```\n'
packet+='\n## Task-private observer delta (absent from production)\n```diff\n'+(v/'parallel-probe.diff').read_text()+'\n```\n'
# Report has no host strings; source is public tracked code only.
for forbidden in ['/private/','/Users/','jesght','63333554','noreply@']:
 assert forbidden not in packet,forbidden
(v/'public-review-packet.md').write_text(packet)
print('head',head,'packet_bytes',len(packet.encode()),'packet_sha256',hashlib.sha256(packet.encode()).hexdigest(),'service_functions',names)
