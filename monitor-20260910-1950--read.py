from pathlib import Path
import json,subprocess,datetime
D=Path(__file__).parent
receipt={'started_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'requests':[]}
def read(name,*args):
 p=subprocess.run(['/opt/homebrew/bin/gh','api',*args],text=True,capture_output=True)
 (D/(name+'.stderr')).write_text(p.stderr)
 receipt['requests'].append({'name':name,'exit':p.returncode})
 (D/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
 if p.returncode:raise RuntimeError('Failed read; no retry: '+p.stderr[:500])
 x=json.loads(p.stdout)
 if isinstance(x,dict) and x.get('errors'):raise RuntimeError('GraphQL errors; no retry')
 (D/(name+'.json')).write_text(json.dumps(x,indent=2)+'\n')
 return x
b=read('budget','rate_limit')['resources']
assert min(b[k]['remaining'] for k in ['core','graphql'])>100
g=read('graph','graphql','--input',str(D/'query.json'))
print(json.dumps(g))
for name,rid in [('mg6486-matrix-run',34518035145),('mg6486-gate-run',34518034956)]:
 x=read(name,f'repos/sparq-org/sparq/actions/runs/{rid}')
 print(name,x['head_sha'],x['status'],x['conclusion'])
main=g['data']['repository']['defaultBranchRef']['target']['oid']
x=read('main-runs',f'repos/sparq-org/sparq/actions/runs?head_sha={main}&event=push&per_page=100')
print('MAIN',main,[(v['id'],v['name'],v['status'],v['conclusion']) for v in x['workflow_runs']])
receipt['completed_at']=datetime.datetime.now(datetime.timezone.utc).isoformat()
(D/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
