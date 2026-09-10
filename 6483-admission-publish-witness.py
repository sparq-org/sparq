from pathlib import Path
import json,subprocess,datetime
A=Path(__file__).parent; R=A/'witness-comment-receipt.json'; assert not R.exists(),'Reconcile existing receipt; do not replay'
r={'at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
def save(): R.write_text(json.dumps(r,indent=2)+'\n')
def api(*args):
 p=subprocess.run(['/opt/homebrew/bin/gh','api',*args],capture_output=True,text=True)
 if p.returncode: raise RuntimeError('Request failed; no retry: '+p.stderr[:500])
 return json.loads(p.stdout)
try:
 b=api('rate_limit')['resources']; assert min(b[k]['remaining'] for k in ['core','graphql'])>=100; r['budget']=b
 main=api('repos/sparq-org/sparq/git/ref/heads/main'); assert main['object']['sha']=='f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464'; r['main']=main
 issue=api('repos/sparq-org/sparq/issues/6483'); assert issue['state']=='open' and not issue['assignees'] and not any(x['name'] in ['needs:user','needs:maintainer','blocked','hold'] for x in issue['labels']); r['issue']=issue
 cs=api('repos/sparq-org/sparq/issues/6483/comments?per_page=100'); assert len(cs)==issue['comments'] and not any('sparq-direct-6483-witness-v1' in c['body'] for c in cs); save()
 r['comment']=api('repos/sparq-org/sparq/issues/6483/comments','-X','POST','--input',str(A/'witness-comment-input.json')); r['completed_at']=datetime.datetime.now(datetime.timezone.utc).isoformat(); save(); print(json.dumps({'issue':6483,'comment':r['comment']['html_url']}))
except Exception as e:
 r['error']=str(e); save(); raise
