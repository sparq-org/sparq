from pathlib import Path
import json,subprocess,datetime,collections
D=Path(__file__).parent
receipt={'started_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'requests':[]}
def read(name,path):
 p=subprocess.run(['/opt/homebrew/bin/gh','api',path],text=True,capture_output=True)
 (D/(name+'.stderr')).write_text(p.stderr)
 receipt['requests'].append({'name':name,'exit':p.returncode});(D/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
 if p.returncode:raise RuntimeError('Read failed; no retry: '+p.stderr[:500])
 x=json.loads(p.stdout);(D/(name+'.json')).write_text(json.dumps(x,indent=2)+'\n');return x
b=read('budget','rate_limit')['resources'];assert min(b[k]['remaining'] for k in ['core','graphql'])>100
main=read('main','repos/sparq-org/sparq/git/ref/heads/main')['object']['sha']
x=read('main-runs',f'repos/sparq-org/sparq/actions/runs?head_sha={main}&event=push&per_page=100')
print('MAIN',main)
print('RUNS',[(v['id'],v['name'],v['status'],v['conclusion']) for v in x['workflow_runs']])
for name,rid in [('ci-jobs',34522399163),('matrix-jobs',34522399200)]:
 x=read(name,f'repos/sparq-org/sparq/actions/runs/{rid}/jobs?per_page=100')
 print(name,dict(collections.Counter(j['conclusion'] or j['status'] for j in x['jobs'])))
 print('FAILURES',[(j['id'],j['name'],j['conclusion']) for j in x['jobs'] if j['conclusion'] not in [None,'success','skipped','neutral']])
for status in ['queued','in_progress']:
 x=read(status,f'repos/sparq-org/sparq/actions/runs?status={status}&per_page=1');print('BACKLOG',status,x['total_count'])
x=read('registry-permissions','repos/jeswr/agent-account-registry/actions/permissions');assert x['enabled'] is False;print('REGISTRY_DISABLED',not x['enabled'])
receipt['completed_at']=datetime.datetime.now(datetime.timezone.utc).isoformat();(D/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
