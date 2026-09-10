from pathlib import Path
import subprocess,json,datetime
A=Path(__file__).parent;H='fe3284199db0831f353d2ae401b8c69472764904';T='PRRT_kwDOSz3qKM6hHXbJ';C=3980166727
receipt={'started_at':datetime.datetime.now(datetime.timezone.utc).isoformat()};P=A/'copilot-duplicate-reply-receipt.json';assert not P.exists(),'Reconcile existing reply before any repeat'
def save():P.write_text(json.dumps(receipt,indent=2)+'\n')
def api(*args):
 p=subprocess.run(['/opt/homebrew/bin/gh','api',*args],capture_output=True,text=True)
 if p.returncode:raise RuntimeError('Request failed; no retry: '+p.stderr[:500])
 j=json.loads(p.stdout);assert not (isinstance(j,dict) and j.get('errors')),j.get('errors') if isinstance(j,dict) else None;return j
try:
 b=api('rate_limit')['resources'];assert b['core']['remaining']>=20 and b['graphql']['remaining']>=20;receipt['budget']=b
 d=api('graphql','--input',str(A/'pr6482-review-watch-query.json'));p=d['data']['repository']['pullRequest'];assert p['state']=='OPEN' and not p['isDraft'] and p['headRefOid']==H and not p['reviewThreads']['pageInfo']['hasNextPage']
 t=[x for x in p['reviewThreads']['nodes'] if x['id']==T];assert len(t)==1 and not t[0]['isOutdated'];assert not t[0]['comments']['pageInfo']['hasNextPage'];cs=t[0]['comments']['nodes'];assert any(c['databaseId']==C and c['author']['login']=='copilot-pull-request-reviewer' for c in cs);assert not any(c['author']['login']=='jeswr' for c in cs),'Own reply already exists; reconcile'
 receipt['pre_reply']=d;save();reply=api(f'repos/sparq-org/sparq/pulls/6482/comments/{C}/replies','-X','POST','--input',str(A/'copilot-duplicate-reply-input.json'));assert reply['in_reply_to_id']==C;receipt['reply']=reply;receipt['thread_resolution']='Left unresolved pending ordinary CI evidence; source discrepancy addressed and followup6483 linked';receipt['completed_at']=datetime.datetime.now(datetime.timezone.utc).isoformat();save();print(json.dumps({'url':reply['html_url'],'thread_resolved':False}))
except Exception as e:receipt['error']=str(e);save();raise
