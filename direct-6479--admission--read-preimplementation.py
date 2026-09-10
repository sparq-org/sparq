from pathlib import Path
import subprocess,json,datetime
A=Path(__file__).parent
assert not (A/'preimplementation-graph.json').exists()
def read(name,*args):
 p=subprocess.run(['/opt/homebrew/bin/gh','api',*args],capture_output=True,text=True)
 (A/('preimplementation-'+name+'.stderr')).write_text(p.stderr)
 if p.returncode:raise RuntimeError('GitHub read failed; no retry: '+p.stderr[:500])
 d=json.loads(p.stdout)
 if isinstance(d,dict) and d.get('errors'):raise RuntimeError('GraphQL errors; no retry')
 (A/('preimplementation-'+name+'.json')).write_text(json.dumps(d,indent=2)+'\n')
 return d
b=read('budget','rate_limit')['resources'];assert min(b[k]['remaining'] for k in ['core','graphql'])>150
d=read('graph','graphql','--input',str(A/'preimplementation-query.json'))
r=d['data']['repository'];print('MAIN',r['defaultBranchRef']['target']['oid']);print('ISSUE',r['issue']['state'],r['issue']['assignees'])
prs=r['pullRequests'];print('OPEN',prs['totalCount'],'PAGING',prs['pageInfo'])
paths={'crates/sparq-engine/src/lib.rs','crates/sparq-engine/src/exec.rs','crates/sparq-engine/src/cache.rs','crates/sparq-engine/src/explain.rs','crates/sparq-engine/src/explain_json.rs'}
for p in prs['nodes']:
 hit=paths.intersection(x['path'] for x in p['files']['nodes'])
 if hit or p['files']['pageInfo']['hasNextPage']:print(json.dumps({'number':p['number'],'title':p['title'],'head':p['headRefOid'],'draft':p['isDraft'],'labels':p['labels'],'hits':sorted(hit),'filepage':p['files']['pageInfo']}))
