import pathlib,json,re,hashlib,subprocess,collections,datetime
p=pathlib.Path(__file__).parent;r=p.parents[1];w=r/'worktrees/issue6483'
HEAD='0b4554b924a80432cc1b572bd19f8e58cdbfb4e6';BASE='f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464';FILE='crates/sparq-bench/src/update_fuzz.rs'
def git(*args):return subprocess.check_output(['git','-C',str(w),*args])
def sha(b):return hashlib.sha256(b).hexdigest()
def dump(n,x):(p/n).write_text(json.dumps(x,indent=2,ensure_ascii=False)+'\n')
def log(n):return (p/(n+'.txt')).read_text().splitlines()
def clean(s):return re.sub(r'\x1b\[[0-9;]*m','',s)
def excerpt(n,indices):
 lines=log(n);(p/(n+'.excerpt.txt')).write_text('\n'.join(str(i+1)+':'+clean(lines[i]) for i in sorted(set(indices)) if 0<=i<len(lines))+'\n')
source=git('show',HEAD+':'+FILE);assert source==(w/FILE).read_bytes()
expected=re.findall(r'#\[test\]\s*fn\s+(\w+)\(',source.decode());assert len(expected)==26
observed=[];all_pass=[]
for name in ['bulk1-log','bulk2-log','bulk3-log']:
 indices=[]
 for i,line in enumerate(log(name)):
  line=clean(line)
  if 'PASS' in line:all_pass.append({'log':name+'.txt','line':i+1,'text':line})
  m=re.search(r'PASS .* update_fuzz::tests::(\w+)\s*$',line)
  if m:observed.append({'name':m[1],'log':name+'.txt','line':i+1,'text':line})
  if m or any(x in line for x in ['Starting 22','Summary [','git log -1','HEAD is now','573dba455effaa982b60776f9b03e6e324d6cc2f','Expected Digest:','SHA256 digest of downloaded','cargo nextest run','FILTERSET:']):indices.append(i)
 excerpt(name,indices)
assert sorted(o['name'] for o in observed)==sorted(expected)
assert len(all_pass)==66
newtests=[n for n in expected if n.startswith('raw_duplicate_')];assert len(newtests)==6
module={'expected_from_exact_source':expected,'expected_count':26,'observed':observed,'all26_once':True,'new6':newtests,'fixed_window_smoke_pass':True,'total_selected_bulk_passes':66,'extraction':'Only ANSI SGR removed from original lines; full test names and PASS required. Three distinct successful job logs, no skipped rows counted.'}
dump('module-tests.json',module)
fullcheckouts={}
for n in ['bulk1-log','bulk2-log','bulk3-log','archive-log','clippy-log','msrv-log','privacy-log','fuzz-log','gate-log']:
 ls=log(n);found=[]
 for i,l in enumerate(ls):
  if 'git log -1 --format=%H' in l:
   m=re.search(r'\b[0-9a-f]{40}\b',ls[i+1]);
   if m:found.append({'sha':m[0],'line':i+2})
 fullcheckouts[n]=found
 assert found and all(x['sha']=='573dba455effaa982b60776f9b03e6e324d6cc2f' for x in found)
commit=json.loads((p/'checkout-commit.json').read_text());tree=git('rev-parse',HEAD+'^{tree}').decode().strip();assert commit['tree']['sha']==tree
paths=git('diff','--name-only',BASE,HEAD).decode().splitlines();assert paths==[FILE]
allow='bench/differential-divergences.json';assert git('show',HEAD+':'+allow)==git('show',BASE+':'+allow)
pr=json.loads((p/'pr-current.json').read_text());assert pr['head']['sha']==HEAD and pr['base']['sha']==BASE
identity={'head':HEAD,'base':BASE,'checkout_sha':commit['sha'],'checkout_parents':[x['sha'] for x in commit['parents']],'reviewed_tree':tree,'checkout_tree':commit['tree']['sha'],'whole_tree_equal':True,'full_checkout_from_each_actual_log':fullcheckouts,'changed_paths':paths,'module_sha256':sha(source),'allowlist_sha256':sha(git('show',HEAD+':'+allow)),'allowlist_unchanged':True,'working_tree_status':git('status','--porcelain').decode(),'pr_open_nondraft':pr['state']=='open' and not pr['draft']}
dump('identity-proof.json',identity)
(p/'full.diff').write_bytes(git('diff',BASE,HEAD,'--',FILE))
(p/'module-source.rs').write_bytes(source)
ctx=[];sourcehash=[]
ranges={'.github/workflows/ci.yml':[(500,523),(551,560),(1356,1415),(1432,1442),(1551,1570)],'.github/workflows/docs-quality.yml':[(145,166),(280,300),(386,409)],'.github/workflows/differential-update.yml':[(54,81),(176,190)],'AGENTS-worker-core.md':[(22,32)],'rust-toolchain.toml':[(1,30)]}
for f,rs in ranges.items():
 b=git('show',HEAD+':'+f);ls=b.decode().splitlines();sourcehash.append({'path':f,'sha256':sha(b),'blob':git('rev-parse',HEAD+':'+f).decode().strip()})
 for a,z in rs:ctx.append('## '+f+':'+str(a)+'\n'+'\n'.join(str(i+1)+': '+ls[i] for i in range(a-1,min(z,len(ls)))))
(p/'workflow-context.txt').write_text('\n\n'.join(ctx)+'\n');dump('source-hashes.json',sourcehash)
# Compact exact raw excerpts; avoid broad formatter diff in derived packet.
for n in ['archive-log','clippy-log','msrv-log']:
 indices=[]
 for i,l in enumerate(log(n)):
  c=clean(l)
  if any(x in c for x in ['##[group]Run cargo','Finished `','Compiling sparq-bench','Checking sparq-bench','Doc-tests sparq_bench','Artifact nextest-archive','SHA256 digest of uploaded','RUSTDOCFLAGS:','rustfmt 1.','rustc 1.88','Process completed with exit code','573dba455effaa982b60776f9b03e6e324d6cc2f','HEAD is now','rustc --version ->']):indices.append(i)
 excerpt(n,indices)
excerpt('privacy-log',list(range(590,616))+list(range(746,748))+list(range(776,783))+list(range(873,879)))
excerpt('select-log',list(range(974,994)))
excerpt('gate-log',list(range(178,183))+list(range(350,362)))
excerpt('fuzz-log',list(range(550,578))+[i for i,l in enumerate(log('fuzz-log')) if re.search(r'fuzz\[[^]]+\] seeds',l)])
ci=json.loads((p/'ci-jobs.json').read_text());matrix=json.loads((p/'matrix-jobs.json').read_text());docs=json.loads((p/'docs-jobs.json').read_text());gate=json.loads((p/'gate-check.json').read_text())
assert len(ci['jobs'])==ci['total_count'];assert len(matrix['jobs'])==matrix['total_count']
summaries={}
for name,x in [('ci',ci),('matrix',matrix),('docs',docs)]:
 summaries[name]={'total':x['total_count'],'states':dict(collections.Counter((j['status']+':'+str(j['conclusion'])) for j in x['jobs'])),'jobs':[{'id':j['id'],'name':j['name'],'status':j['status'],'conclusion':j['conclusion']} for j in x['jobs']]}
stepdata=[{'job_id':j['id'],'name':j['name'],'steps':j['steps']} for j in ci['jobs']+docs['jobs'] if j['id'] in [103009171977,103010136170,103008605738,103009171614]]
dump('job-summary.json',summaries);dump('relevant-step-results.json',stepdata)
cl=clean((p/'clippy-log.txt').read_text());formatter={'whole_workspace_check':'executed, exit 1; nonblocking by existing ci.yml1432-1441','version':'rustfmt1.9.0-stable (8bab26f4f6 2026-07-14)','changed_file_diagnostic_occurrences':cl.count('crates/sparq-bench/src/update_fuzz.rs'),'changed_file_interpretation':'No formatting diff for the touched file appears in the complete formatter output. This is not reported as a whole-workspace fmt PASS. No new local formatter run.'};assert formatter['changed_file_diagnostic_occurrences']==0
report={'purpose':'PR6487 current Linux CI execution evidence only; no code review or publication authorization','author':'GPT-6 Astra xhigh','head':HEAD,'result':'READY_FOR_ROOT_CI_ASSESSMENT_WITH_EXPLICIT_SCOPE_LIMITS','identity':identity,'module_tests':{'passed':26,'new_passed':6,'fixed_window_smoke':'PASS in bulk2 job103014142251 log536','bulk_job_ids':[103014142138,103014142251,103014142149],'selected_tests_total':66,'scope':'Selection affected only sparq-bench (1/68 members); 22 tests each bulk shard. Not an executed full-workspace unit/integration test sweep.'},'workspace_compile':'Successful cargo nextest archive --workspace --all-targets --features approx-ann,filtered-ann,vec-predicate, archive artifact10169127229 digest908dc8bb1be24b9c2fea8e051f1acb1fa6e9ccb2a122a73e8747186cb6124eea consumed by all3 shards; workspace doctests also successful. No local archive downloaded.','clippy':'Successful workspace --all-targets -Dwarnings default and --all-features; exact job103009171977. Workspace rustdoc default/all-features -Dwarnings also successful.','msrv':'Actual Rust1.88.0 cargo check --workspace --all-targets --exclude sparq-py --exclude sparq-hdt success; sparq-bench checked. Job103009171614.','formatter':formatter,'privacy':'Actual Linux bash scripts/check-privacy-claims.sh succeeded over784files; its32 selftests passed. Job103008605738.','preflight_qualification':'CI executes preflight.py --self-test and test_preflight.py, not the author preflight runner against the actual PR diff. Nested printed FAIL/PASS examples belong to hermetic selftests; they are not actual PR preflight findings. The prior local Bash3 author-preflight limitation is not represented as a full Linux author-preflight execution. Actual delegated privacy gate is separately proven.','gate':{'id':gate['id'],'app_id':gate['app']['id'],'head':gate['head_sha'],'status':gate['status'],'conclusion':gate['conclusion'],'completed_at':gate['completed_at'],'run':34518125361,'tier':'full','stable_gating_total':64,'ran':41,'skipped':23,'advisory_excluded':6},'ci':summaries['ci']['states'],'matrix':summaries['matrix']['states'],'fuzz':'Completed differential smoke job103010284347 runs QUERY fuzz fixed windows; this is distinct from UPDATE fixed_window_smoke proved in bulk. cargo-fuzz randomized parser/mmap leg skipped. No nightly UPDATE soak evidence was requested/fetched.','remaining_limits':['Normal nightly differential-update.yml schedule/manual soak not observed on this candidate. No seed replay performed in this task.','Conformance/wasm CI jobs selected out for bench-only change; their skipped status is not execution proof. Matrix detailed per-leg logs not fetched; job-level metadata only.','Workspace fmt informational check failed on existing broad formatting debt; no touched-file diagnostic observed.','Full Linux author-preflight execution on actual diff not present; its selftests and actual privacy delegate are proven.','Copilot/root model review and queue actions outside this evidence audit.'],'requests':{'count':len((p/'requests.jsonl').read_text().splitlines()),'cap':20,'initial_core_remaining':json.loads((p/'budget.json').read_text())['resources']['core']['remaining'],'errors':0,'retries':0,'live_job_logs_requested':0},'local_tools':'Read-only Git/source parsing and artifact writes only. Initial excerpt script SyntaxError corrected without network reread; saved local-excerpt-correction.json. No builds/tests/formatting/remote mutation.','commands_pending':False}
dump('report.json',report)
packet='PR6487 bounded CI evidence\n\n'+json.dumps(report,indent=2,ensure_ascii=False)+'\n\nExact 26 observed tests\n'+ '\n'.join(o['text'] for o in observed)+'\n\nSource and workflow qualification\n'+(p/'workflow-context.txt').read_text()
(p/'packet.txt').write_text(packet)
files=[]
for f in sorted(p.rglob('*')):
 if f.is_file() and f.name!='manifest.json':files.append({'path':str(f.relative_to(p)),'bytes':f.stat().st_size,'sha256':sha(f.read_bytes())})
dump('manifest.json',{'files':files,'count':len(files),'total_bytes':sum(f['bytes'] for f in files)})
print(json.dumps({'tests':len(observed),'newtests':newtests,'ci':report['ci'],'matrix':report['matrix'],'head':HEAD,'tree':tree,'gate':report['gate'],'files':len(files),'bytes':sum(f['bytes'] for f in files),'manifest_sha256':sha((p/'manifest.json').read_bytes()),'packet_bytes':len(packet.encode()),'packet_sha256':sha(packet.encode())},indent=2))
