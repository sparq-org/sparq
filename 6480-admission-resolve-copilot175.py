from pathlib import Path
import json,subprocess,datetime
A=Path(__file__).parent;HEAD='17594d4a6534c142cae764772fc42049e898eca3';THREAD='PRRT_kwDOSz3qKM6hCljp';COMMENT=3978298773
receipt={'started_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
def save(): (A/'resolve-copilot175-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
def api(*args):
 p=subprocess.run(['/opt/homebrew/bin/gh','api',*args],text=True,capture_output=True)
 if p.returncode: raise RuntimeError('GitHub request failed; no retry: '+p.stderr[:500])
 d=json.loads(p.stdout)
 if isinstance(d,dict) and d.get('errors'): raise RuntimeError('GraphQL error; no retry: '+str(d['errors'])[:500])
 return d
def fresh():
 d=api('graphql','--input',str(A/'final175-target-query.json'));r=d['data']['repository']['pullRequest'];assert r['state']=='OPEN' and not r['isDraft'] and r['headRefOid']==HEAD and r['baseRefName']=='main';assert r['mergeQueueEntry'] is None;assert not r['reviewThreads']['pageInfo']['hasNextPage'];assert len(r['reviews']['nodes'])<40 and all(x['state']!='CHANGES_REQUESTED' for x in r['reviews']['nodes']);return d,r
try:
 assert not (A/'resolve-copilot175-receipt.json').exists(),'Inspect existing mutation receipt; do not replay'
 b=api('rate_limit')['resources'];assert min(b[k]['remaining'] for k in ['core','graphql'])>100;receipt['budget']=b
 proof=json.loads((A/'head175-final-ci-verification.json').read_text());assert proof['head']==HEAD and proof['all_required_validation_passed'] and proof['copilot_critical_refuted']
 expected=json.loads((A/'copilot175-inline-comments.json').read_text());assert len(expected)==1
 c=api(f'repos/sparq-org/sparq/pulls/comments/{COMMENT}');assert c['commit_id']==HEAD and c['body']==expected[0]['body'] and c['user']['login']==expected[0]['user']['login']
 d,r=fresh();t=[x for x in r['reviewThreads']['nodes'] if x['id']==THREAD];assert len(t)==1 and not t[0]['isResolved'] and not t[0]['isOutdated'];receipt['pre_reply']=d;save()
 reply=api(f'repos/sparq-org/sparq/pulls/6481/comments/{COMMENT}/replies','-X','POST','--input',str(A/'copilot175-reply-input.json'));receipt['reply']=reply;save();assert reply['in_reply_to_id']==COMMENT
 d,r=fresh();t=[x for x in r['reviewThreads']['nodes'] if x['id']==THREAD];assert len(t)==1 and not t[0]['isResolved'];receipt['pre_resolve']=d;save()
 payload={'query':'mutation($id:ID!){resolveReviewThread(input:{threadId:$id}){thread{id isResolved}}}','variables':{'id':THREAD}};(A/'resolve-copilot175-input.json').write_text(json.dumps(payload)+'\n')
 result=api('graphql','--input',str(A/'resolve-copilot175-input.json'));receipt['resolve']=result;save();assert result['data']['resolveReviewThread']['thread']['isResolved']
 d,r=fresh();assert all(x['isResolved'] for x in r['reviewThreads']['nodes']);assert r['body']==(A/'pr-body-sharp-followup.md').read_text();receipt['pre_body']=d;save()
 body=api('repos/sparq-org/sparq/pulls/6481','-X','PATCH','--input',str(A/'pr175-ci-green-input.json'));assert body['head']['sha']==HEAD;receipt['body']=body;receipt['completed_at']=datetime.datetime.now(datetime.timezone.utc).isoformat();save();print(json.dumps({'reply':reply['html_url'],'resolved':THREAD,'body_updated':True,'head':HEAD}))
except Exception as e:
 receipt['error']=str(e);save();raise
