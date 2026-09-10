from pathlib import Path
import json,hashlib,re,subprocess,os
p=Path(__file__).resolve().parent;r=p.parent.parent;w=r/'worktrees/issue5183';old=p.parent/'original-seed-execution';sha=lambda b:hashlib.sha256(b).hexdigest()
def put(n,v):(p/n).write_text(json.dumps(v,indent=2)+'\n' if not isinstance(v,str) else v)
protocol=json.loads((p/'protocol.json').read_text());commands=json.loads((p/'commands.json').read_text());phase=json.loads((p/'phase-result.json').read_text());resources=json.loads((p/'resources.json').read_text());ident=json.loads((p/'readiness-identities-and-cache.json').read_text())
assert phase['status']=='complete' and len(commands)==16 and sum('seed-' in c['name'] for c in commands)==14
(p/'outcomes').mkdir(exist_ok=True);results=[]
for seed in protocol['seeds']:
 rows=[]
 for rev in ['parent','main']:
  name=f'{rev}-seed-{seed}';stdout=(p/(name+'.stdout')).read_text();stderr=(p/(name+'.stderr')).read_text();c=next(c for c in commands if c['name']==name)
  first=stdout.split('FIRST FAILING CASE:\n',1)[1] if 'FIRST FAILING CASE:\n' in stdout else None
  sequence=first.split('--- full sequence ---\n',1)[1] if first and '--- full sequence ---\n' in first else None
  row={'revision':rev,'source_revision':protocol['revisions'][rev],'seed':seed,'exit':c['exit'],'stdout_sha256':sha(stdout.encode()),'stderr_sha256':sha(stderr.encode()),'sequence_origin':'Deterministically reconstructed by unchanged original generator, not captured raw CI sequence.','startup_allowlist_enabled': 'adjudicated classes enabled ["update-oxigraph-integer-lexical-canonicalization"]; every other divergence fails' in stdout}
  if first:
   m=re.search(r'^step=(\d+) of (\d+)\nop: (.*?)\n(canonical dataset differs[^\n]*)\n',first,re.M|re.S);assert m
   row.update(failing_index_zero_based=int(m[1]),request_count=int(m[2]),failing_operation=m[3],reason=m[4]);detail=first[first.index('canonical dataset differs'):first.index('\nrepro:')]
   row['failure_detail_sha256']=sha(detail.encode());row['first_failure_body_sha256']=sha(first.encode());put(f'outcomes/{rev}-{seed}-failure.txt',detail)
   only_a=detail.split('only in sparq(rebuild):\n',1)[1].split('only in oxigraph:\n',1)[0];only_b=detail.split('only in oxigraph:\n',1)[1];a=[x.strip() for x in only_a.splitlines() if x.strip()];b=[x.strip() for x in only_b.splitlines() if x.strip()]
   row.update(only_in_sparq=a,only_in_oxigraph=b,net_extra_reported_quads=len(a)-len(b),net_extra_bnode_bearing_quads=sum('_:' in x for x in a)-sum('_:' in x for x in b))
  if sequence:
   starts=list(re.finditer(r'^\[(\d+)\] ',sequence,re.M));ops=[sequence[x.end():starts[i+1].start() if i+1<len(starts) else len(sequence)].rstrip() for i,x in enumerate(starts)];assert [int(x[1]) for x in starts]==list(range(row['request_count']))
   row['sequence_sha256']=sha(sequence.encode());row['load_reference_mirrors']=[x for x in ops if '(reference engine ran:' in x];row['request_indices_with_both8_and008']=[i for i,x in enumerate(ops) if '<http://ex/s2> <http://ex/p1> 8 .' in x and '<http://ex/s2> <http://ex/p1> "008"' in x]
   row['requests_after_failure_not_executed']=row['request_count']-row['failing_index_zero_based']-1
   put(f'outcomes/{rev}-{seed}-sequence.txt',sequence);put(f'outcomes/{rev}-{seed}-requests.json',ops)
  rows.append(row)
 pair={'seed':seed,'parent':rows[0],'main':rows[1],'same_exit':rows[0]['exit']==rows[1]['exit'],'same_failure_body':rows[0].get('first_failure_body_sha256')==rows[1].get('first_failure_body_sha256'),'same_sequence':rows[0].get('sequence_sha256')==rows[1].get('sequence_sha256')};results.append(pair)
assert all(x['same_exit'] and x['same_failure_body'] and x['same_sequence'] for x in results)
put('results.json',results)
# Preserve relevant historical executed dependency compiler flags without copying broad logs.
prior=json.loads((old/'compiler-linked-provenance.json').read_text());features={}
for rev in ['parent','main']:
 records=[]
 for rec in prior[rev]:
  argv=rec['actual_rustc_argv'];cfg=[argv[i+1] for i,x in enumerate(argv[:-1]) if x=='--cfg'];records.append({'crate':rec['crate'],'actual_rustc_argv':argv,'features':cfg})
 features[rev]={'recorded_compiler_records':records,'metadata_feature_proof_file':rev+'-setup-proof.json','direct_extern_hash_revalidated_each_command':True}
put('dependency-feature-provenance.json',features)
# Recorded bytecode identities remain unchanged; candidate is clean and untouched.
head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=w).decode().strip();status=subprocess.check_output(['git','status','--porcelain'],cwd=w).decode();assert head==protocol['candidate_untouched'] and status==''
for v in ident['frozen_binaries'].values():assert sha(Path(v['binary']).read_bytes())==v['sha256'];assert sha(Path(v['source_module']).read_bytes())==v['module_sha256']
put('final-source-verification.json',{'candidate_head':head,'candidate_clean':True,'original_binaries_and_module_still_match':True,'no_candidate_source_import':True})
resources_summary={'phase_seconds':phase['seconds'],'sum_command_seconds':sum(c['seconds'] for c in commands),'driver_build_seconds':{c['name']:c['seconds'] for c in commands if c['name'].endswith('-build')},'max_monitored_new_allocated_bytes':max(v['new_allocated'] for v in resources),'min_monitored_free_bytes':min(v['free_bytes'] for v in resources),'final_free_bytes':os.statvfs(p).f_bavail*os.statvfs(p).f_frsize,'bounds':protocol['limits']}
put('resource-summary.json',resources_summary)
report={'issue':5183,'author':'GPT-6 Astra xhigh','decision':'All seven original CI seed markers reproduced once on both recorded revisions; no new implementation or candidate-generator run.','results':[{'seed':x['seed'],'index_zero_based':x['main']['failing_index_zero_based'],'requests':x['main']['request_count'],'parent_exit':x['parent']['exit'],'main_exit':x['main']['exit'],'same_failure_and_sequence':x['same_failure_body'] and x['same_sequence'],'sequence_sha256':x['main']['sequence_sha256']} for x in results],'common_observation':'Every case fails comparing sparq(rebuild) to Oxigraph after INSERT { ?s <http://ex/p0> _:bt } WHERE { ?s <http://ex/p1> ?o }. Each preceding sequence explicitly inserts distinct8 and008 terms at s2/p1. Deltas show lexical008 plus one net extra bnode-bearing quad on Sparq; some canonical labels also shift.','inference':'All seven local witnesses are consistent with the already-reproduced reference lexical-collapse/WHERE-cardinality mechanism of first seed4141222487. This phase does not add separate per-step WHERE cardinality instrumentation or prove every hypothetical alternate cause; strict comparator failure is preserved.','lineage':'Identical original source0807388f, hash-verified d41 and781f dependencies; equal entire failure bodies and reconstructed sequences in both revisions. No evidence that6478 introduced these seven failures.','ci_qualification':'Frozen raw CI only contained these seven MISMATCH seed markers. Indices, request sequences and printed LOAD mirrors here are newly deterministically reconstructed original-generator evidence, not recovered raw CI sequence text.','driver_qualification':'New Rust2021 drivers match workspace edition; older diagnostic driver used2024. Unique driver/metadata/output paths and fixed-list CLI replace old hard-coded first seed, while original module and recorded dependency flags/features remain unchanged. macOS O3/unwind/noLTO/codegen16/Rayon1 differs from LinuxCI release-fast.','limits':['No candidate generator, seed search or outcome retries. Every seed was run once per revision; all14 exited1.','Original run stops at first mismatch. Printed suffix requests were not executed; per-case count is in results.json.','LOAD reference INSERT mirrors are printed and captured. Original sandbox behavior is unchanged; ephemeral LOAD files are not separately retained after its normal cleanup.','These failures do not establish an engine fix, a comparator defect or passing all historical samples with candidate. Future source admission remains subject to independent review/CI.'],'resources':resources_summary,'terminal':'Two builds and14 seed processes returned terminal codes; controller returned0. No managed command pending.','next_small_step':'Root can fold these fixed-list observations into the existing5183 diagnosis/review. The repeated collision/template shape supports grouping the seven with the first reference-domain failure, with the stated instrumentation limit. No further replay or production edit is required by this phase.'}
put('report.json',report)
packet='# Original remaining seven UPDATE failures: fixed-list execution\n\nActual author: GPT-6 Astra (xhigh). No production changes.\n\n```json\n'+json.dumps(report,indent=2)+'\n```\n\n## Exact original module and binary provenance\n\nOriginal module SHA2560807388f84792f011b868456269a36bf74e5e463c67dfb71d1be3ad9d35aecf7 is byte-identical at both d41ec9fcb796504d85f5a5247a5fdc9eb3e65de5 and781f667c19a8ebb779cfccb24b05ea432360b025. Each new driver used only its recorded direct extern artifacts, rehashed before every command. No Cargo or dependency build ran.\n\n## Per-seed exact localized detail\n'
for x in results:
 seed=x['seed'];packet+='\n### Seed'+str(seed)+' (both revisions identical)\n\n```text\n'+(p/f'outcomes/main-{seed}-failure.txt').read_text()+'\n```\n'
packet+='\n## Evidence scope\n\nThe bundle retains both per-revision raw stdout/stderr, new driver binaries/source/actual compiler argv, immutable original source, per-command identities, printed sequences and LOAD mirrors, feature proofs, resources and final clean-candidate proof. This compact packet omits long request bodies; their exact hashes and complete files are in outcomes/. Original compiler environment/build diagnostics are kept locally rather than embedded here.\n'
assert '/Users/' not in packet and '/private/tmp/' not in packet
put('review-packet.md',packet)
put('terminal-receipt.json',{'commands':16,'builds':2,'seed_runs':14,'controller_exit':0,'all_command_receipts_terminal':all('exit' in c for c in commands),'no_retry':True})
files=[]
for q in sorted(p.rglob('*')):
 if not q.is_file() or q.name in ('manifest.json','manifest.sha256'):continue
 assert not q.is_symlink();b=q.read_bytes();files.append({'path':str(q.relative_to(p)),'bytes':len(b),'sha256':sha(b)})
put('manifest.json',{'source_revisions':protocol['revisions'],'files':files});m=sha((p/'manifest.json').read_bytes());put('manifest.sha256',m+'  manifest.json\n')
for x in files:assert sha((p/x['path']).read_bytes())==x['sha256']
print(json.dumps({'files':len(files),'manifest_sha256':m,'packet_bytes':len(packet.encode()),'packet_sha256':sha(packet.encode()),'resources':resources_summary,'results':report['results']},indent=2))
