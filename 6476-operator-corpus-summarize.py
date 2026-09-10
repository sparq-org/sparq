# GPT-6 Astra: exact finite corpus/result validation, no timing inference.
import hashlib,json,re,shutil,subprocess,time
from pathlib import Path
p=Path(__file__).resolve().parent
wt=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue6476')
sha=lambda f:hashlib.sha256(f.read_bytes()).hexdigest()
plan=json.loads((p/'source-plan.json').read_text()); proto=json.loads((p/'protocol.json').read_text())
expected=[Path(x).stem for x in sorted(plan['corpus_sha256'])]
records=[]; loads={}
for mode in proto['modes']:
 receipt=json.loads((p/(mode+'.json')).read_text()); assert receipt['exit_code']==0 and receipt['stop'] is None
 lines=(p/(mode+'.log')).read_text().splitlines(); rowlines=[x for x in lines if x.startswith('RESULT\t')]
 assert [x.split('\t')[2] for x in rowlines]==expected
 assert lines.count('COMPLETED\t'+mode+'\t28')==1
 assert not any('OPERATOR_CORPUS_ERROR:' in x for x in lines)
 loads[mode]=[int(x.split('\t')[1]) for x in lines if x.startswith('LOADED\t')]; assert len(loads[mode])==1
 for line in rowlines:
  parts=line.split('\t'); assert len(parts)==7 and parts[1]==mode
  values=[int(x) for x in parts[4:]]; assert len(set(values))==1
  graph=parts[2] in ['q27_construct','q28_describe']
  unit='triples' if graph else 'json_bytes' if mode=='json' else 'count_result' if mode=='count' else 'solution_rows'
  assert parts[3]==unit
  records.append({'query':parts[2],'mode':mode,'unit':unit,'iterations':values})
assert len(records)==84 and len({(x['query'],x['mode']) for x in records})==84
bykey={(x['query'],x['mode']):x for x in records}
assert all(bykey[(q,'count')]['iterations']==bykey[(q,'materialize')]['iterations'] for q in expected)
assert all(bykey[(q,'count')]['iterations']==bykey[(q,'json')]['iterations'] for q in ['q27_construct','q28_describe'])
assert len({x[0] for x in loads.values()})==1
inv=json.loads((p/'invalid-query.json').read_text()); ilog=(p/'invalid-query.log').read_text()
assert inv['exit_code']==2 and inv['stop'] is None and 'COMPLETED' not in ilog
assert 'OPERATOR_CORPUS_ERROR: count/q13_bind:' in ilog
assert sum(x.startswith('RESULT\t') for x in ilog.splitlines())==12
assert all(sha(p/'corpus'/k)==v for k,v in plan['corpus_sha256'].items())
assert all(sha(wt/k)==v for k,v in plan['engine_source_sha256'].items())
assert subprocess.check_output(['git','status','--porcelain'],cwd=wt,text=True)==''
assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=wt,text=True).strip()==plan['head']
(p/'results-84.json').write_text(json.dumps({'head':plan['head'],'records':records},indent=2)+'\n')
compiled=(p/'compiler-invocations.txt').read_text().splitlines()
deps=[]
for name,path in re.findall(r'--extern ([A-Za-z0-9_]+)=(\S+)',compiled[0]):
 f=Path(path.rstrip('`'));deps.append({'name':name,'file':f.name,'bytes':f.stat().st_size,'sha256':sha(f)})
(p/'engine-dependencies.json').write_text(json.dumps({'actual_compiler_features':re.findall(r'--cfg \'feature="([^"]+)"\'',compiled[0]),'linked_dependencies':deps},indent=2)+'\n')
receipts=[json.loads((p/(x+'.json')).read_text()) for x in ['generate','count','materialize','json','invalid-query']]
def allocated(root):
 total=0
 for x in [root,*root.rglob('*')]:
  try:
   if not x.is_symlink():total+=x.stat().st_blocks*512
  except FileNotFoundError:pass
 return total
free=shutil.disk_usage(p).free
growth=allocated(Path(proto['target']))+sum(allocated(Path(x)) for x in proto['prior_evidence'])+allocated(p)-proto['original_target_initial_allocated_bytes']
assert free>=proto['minimum_free_bytes'] and growth<proto['aggregate_growth_limit_bytes']
report={'head':plan['head'],'decision':'REGISTERED_CORPUS_EXECUTION_COMPLETE','source_clean':True,'scope':'Equivalent2000-entity registered28-query public-API mode corpus; not full CLI benchmark/workflow execution.','records':84,'query_invocations':252,'iterations_each':3,'modes':proto['modes'],'same_iteration_returned_size_all84':True,'count_materialize_size_equal_all28':True,'graph_form_returned_triples_equal_all_modes':True,'json_non_graph_records':26,'graph_records':6,'dataset':{'entities':2000,'generated_nt_lines':sum(1 for _ in (p/'dataset.nt').open()),'loaded_graph_triples_by_mode':loads,'bytes':(p/'dataset.nt').stat().st_size,'sha256':sha(p/'dataset.nt'),'generator_sha256':plan['generator_sha256'],'generator':'verbatim crates/sparq-bench/src/dataset.rs::write_nt'},'negative_control':{'query':'q13_bind.rq replaced only in private copy','exit_code':2,'prior_success_records':12,'completion_marker':False,'exact_query_error_association':True},'build':{'fresh_d07_engine_and_unique_harness_compilation':True,'observed_finished_seconds':32.41,'cargo_exit_code':None,'limitation':'Disk-monitor wrapper failed on a vanished rustc temporary before wait/receipt. Cargo independently completed per actual compiler log; no Cargo exit code recovered and no build repeated. Monitoring gap is preserved in monitor-failure.json.'},'resource':{'final_free_bytes':free,'final_aggregate_growth_bytes':growth,'cap_bytes':proto['aggregate_growth_limit_bytes'],'floor_bytes':proto['minimum_free_bytes'],'max_observed_execution_growth_bytes':max(x['max_growth_bytes'] for x in receipts),'min_observed_execution_free_bytes':min(x['min_free_bytes'] for x in receipts),'phase_elapsed_seconds_at_summary':time.time()-proto['started_epoch'],'jobs':1,'rustc_profile':proto['profile_overrides']},'limitations':['Return-size stability and cross-count agreement are not an independent semantic answer oracle.','JSON byte lengths are not solution counts; CONSTRUCT/DESCRIBE return triple counts even in json mode, matching CLI dispatch.','Default engine features, System allocator and one Rayon worker; CLI dependency-unified feature set and mimalloc were not reproduced.','No timing, allocation, regression, full-workflow or full-feature conformance claim.','Missing corpus names are enforced by actual harness; deliberately missing-name mutation was not separately executed.','Compile resource-monitor observation gap recorded; subsequent runs stayed inside both resource bounds.'],'artifacts':{'results':'results-84.json','provenance':'binary-provenance.json','actual_compiler_argv':'compiler-invocations.txt','dependencies':'engine-dependencies.json','commands':'generate.json,count.json,materialize.json,json.json,invalid-query.json'}}
(p/'report.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
