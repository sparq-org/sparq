from pathlib import Path
import datetime,hashlib,json,subprocess
A=Path(__file__).parent
R=A/'close5183-receipt.json'
assert not R.exists(), 'Reconcile existing receipt; never replay'
r={'started_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
def save(): R.write_text(json.dumps(r,indent=2)+'\n')
def api(*a):
 p=subprocess.run(['/opt/homebrew/bin/gh','api',*a],capture_output=True,text=True)
 if p.returncode: raise RuntimeError('GitHub operation failed; no retry: '+p.stderr[:500])
 return json.loads(p.stdout)
try:
 b=api('rate_limit')['resources']; assert min(b[k]['remaining'] for k in ['core','graphql'])>100; r['budget']=b
 issue=api('repos/sparq-org/sparq/issues/5183')
 prior=json.loads((A/'close5183-issue.json').read_text())
 assert issue['state']=='open' and issue['updated_at']==prior['updated_at'] and issue['body']==prior['body'] and issue['comments']==prior['comments']
 assert not any(x['name'] in ['needs:user','needs:maintainer','hold'] for x in issue['labels'])
 pr=api('repos/sparq-org/sparq/pulls/6482'); assert pr['merged'] and pr['merge_commit_sha']=='f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464' and pr['head']['sha']=='fe3284199db0831f353d2ae401b8c69472764904'
 main=api('repos/sparq-org/sparq/git/ref/heads/main'); assert main['object']['sha']==pr['merge_commit_sha']
 gate=api('repos/sparq-org/sparq/check-runs/102940507965'); assert gate['name']=='gate' and gate['head_sha']==pr['merge_commit_sha'] and gate['conclusion']=='success' and gate['app']['id']==15368
 cs=api('repos/sparq-org/sparq/issues/5183/comments?per_page=100'); assert len(cs)==issue['comments'] and not any('sparq-direct-5183-landed-closure-v1' in c['body'] for c in cs)
 r.update(issue_before=issue,pr=pr,main=main,gate=gate,comment_body_sha256=hashlib.sha256((A/'close5183-body.md').read_bytes()).hexdigest()); save()
 c=api('repos/sparq-org/sparq/issues/5183/comments','-X','POST','--input',str(A/'close5183-comment-input.json')); r['comment']=c; save()
 fresh=api('repos/sparq-org/sparq/issues/5183'); assert fresh['state']=='open' and fresh['body']==issue['body'] and fresh['comments']==issue['comments']+1 and fresh['labels']==issue['labels']
 r['immediate_preclose']=fresh; save()
 closed=api('repos/sparq-org/sparq/issues/5183','-X','PATCH','--input',str(A/'close5183-state-input.json')); r['closed']=closed; save(); assert closed['state']=='closed'
 verify=api('repos/sparq-org/sparq/issues/5183'); assert verify['state']=='closed' and verify['state_reason']=='completed'; r['verification']=verify; r['completed_at']=datetime.datetime.now(datetime.timezone.utc).isoformat(); save()
 print(json.dumps({'issue':5183,'state':verify['state'],'closed_at':verify['closed_at'],'comment':c['html_url']}))
except Exception as e:
 r['error']=str(e); save(); raise
