from pathlib import Path
import json,subprocess,datetime
A=Path(__file__).parent
assert not (A/'issue5183-reference-comment-receipt.json').exists(),'Reconcile existing receipt; do not duplicate a comment'
def api(*args):
 p=subprocess.run(['/opt/homebrew/bin/gh','api',*args],text=True,capture_output=True)
 if p.returncode: raise RuntimeError('GitHub operation failed; no retry: '+p.stderr[:400])
 d=json.loads(p.stdout)
 if isinstance(d,dict) and d.get('errors'): raise RuntimeError(str(d['errors'])[:400])
 return d
b=api('rate_limit')['resources'];assert min(b[x]['remaining'] for x in ['core','graphql'])>100
proof=json.loads((A/'oxigraph-witness-root-verification.json').read_text());assert proof['all_three_cases_checked'] and proof['raw_stdout_equals_observations']
f=api('graphql','--input',str(A/'issue5183-reference-fresh-query.json'));i=f['data']['repository']['issue'];assert i['number']==5183 and i['state']=='OPEN'
(A/'issue5183-reference-precheck.json').write_text(json.dumps({'budget':b,'target':f},indent=2)+'\n')
existing=[x for x in i['comments']['nodes'] if 'sparq-direct-5183-reference-reduction-v1' in x['body']]
if existing:
 out={'at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'existing':existing,'action':'No duplicate comment posted'}
else:
 result=api('repos/sparq-org/sparq/issues/5183/comments','-X','POST','--input',str(A/'issue5183-reference-comment-input.json'));out={'at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'created':result}
(A/'issue5183-reference-comment-receipt.json').write_text(json.dumps(out,indent=2)+'\n');print(json.dumps({'url':out.get('created',{}).get('html_url') or out['existing'][0]['url'],'action':'created' if 'created' in out else 'existing'}))
