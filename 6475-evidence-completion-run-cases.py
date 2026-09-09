# [GPT-6 Astra] Predeclared finite matrix and full packaged manifest; no retries.
from pathlib import Path
import json,subprocess,shutil,time,itertools,collections
E=Path(__file__).resolve().parent;R=E/'target/debug/canon-relabel-replay';N='/Users/jesght/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node'
start=time.monotonic();records=[]
protocol={'suite':'all86 entries of packaged rdf-canon0.15.3 tests/manifest.jsonld','rust':'upstream defaults:4000 global HNDQ calls','js_modes':['default work factor1','diagnostic fixed4000 deep iterations'],'js_case_timeout_ms':1000,'subprocess_timeout_seconds':2,'whole_execution_cap_seconds':120,'floor_bytes':6*1024**3,'retries':0,'note':'JS baseline default work factor is retained; an explicit1s case timeout is added and separately reported.'}
(E/'protocol.json').write_text(json.dumps(protocol,indent=2)+'\n')
def call(runtime,file,mode='output',hash='SHA256',budget='default'):
 assert shutil.disk_usage(E).free>=6*1024**3
 assert time.monotonic()-start<120
 argv=([str(R)] if runtime=='rust' else [N,str(E/'js-case.cjs')])+[str(file),mode,hash,budget]
 t=time.monotonic()
 try:
  p=subprocess.run(argv,cwd=E,capture_output=True,text=True,timeout=2)
  row={'argv':argv,'exit':p.returncode,'stdout':p.stdout,'stderr':p.stderr,'seconds':time.monotonic()-t}
  try:row['result']=json.loads(p.stdout)
  except Exception:row['result']={'harness_error':'non-JSON output'}
 except subprocess.TimeoutExpired as e:
  row={'argv':argv,'exit':None,'seconds':time.monotonic()-t,'result':{'external_timeout_seconds':2}}
 records.append(row)
 (E/'commands-cases.json').write_text(json.dumps(records,indent=2)+'\n')
 return row['result']
(E/'inputs').mkdir()
matrix=[]
for label in ['original','renamed']:
 lines=(E.parent/'replay'/(label+'.nq')).read_text().strip().splitlines()
 for order in itertools.permutations(range(3)):
  f=E/'inputs'/(label+'-'+''.join(map(str,order))+'.nq');f.write_text('\n'.join(lines[i] for i in order)+'\n')
  matrix.append({'labeling':label,'order':list(order),'input':f.read_text(),'result':call('rust',f)})
(E/'rust-matrix.json').write_text(json.dumps(matrix,indent=2)+'\n')
budgets=[]
for label in ['original','renamed']:
 for limit in [0,1,2,3,4]:budgets.append({'labeling':label,'limit':limit,'result':call('rust',E/'inputs'/(label+'-012.nq'),budget=str(limit))})
(E/'rust-budget-cases.json').write_text(json.dumps(budgets,indent=2)+'\n')
manifest=json.loads((E/'fixtures/manifest.jsonld').read_text());suite=[]
for i,entry in enumerate(manifest['entries']):
 mode='map' if entry['type']=='rdfc:RDFC10MapTest' else 'output'
 expected=None
 if entry.get('result'):
  s=(E/'fixtures'/entry['result']).read_text();expected=json.loads(s) if mode=='map' else s
 for runtime,budget in [('rust','default'),('js','default'),('js','4000')]:
  result=call(runtime,E/'fixtures'/entry['action'],mode,entry.get('hashAlgorithm','SHA256'),budget)
  if entry['type']=='rdfc:RDFC10NegativeEvalTest':
   passed=result.get('error_debug','').startswith('HndqCallLimitExceeded') if runtime=='rust' else result.get('error','').startswith('Maximum deep iterations exceeded')
  else:passed='value' in result and result['value']==expected
  suite.append({'id':entry['id'],'type':entry['type'],'name':entry['name'],'action':entry['action'],'runtime':runtime,'budget':budget,'expected':expected,'result':result,'pass':passed})
 (E/'suite-results.json').write_text(json.dumps(suite,indent=2)+'\n')
 if i%20==0:print('fixtures_completed',i+1,flush=True)
summary={}
for runtime,budget in [('rust','default'),('js','default'),('js','4000')]:
 rows=[x for x in suite if x['runtime']==runtime and x['budget']==budget]
 summary[runtime+'-'+budget]={'total':len(rows),'passed':sum(x['pass'] for x in rows),'failed_or_limited':[{k:x[k] for k in ['id','type','result']} for x in rows if not x['pass']]}
(E/'suite-summary.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps(summary,indent=2),flush=True)
(E/'run-finish.json').write_text(json.dumps({'elapsed_seconds':time.monotonic()-start,'free_bytes':shutil.disk_usage(E).free,'command_count':len(records)},indent=2)+'\n')
