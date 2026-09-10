from pathlib import Path
import json,subprocess,datetime,hashlib
R=Path('/private/tmp/sparq-pr6049/.throughput-monitor'); A=R/'direct-6485/admission'; W=R/'worktrees/issue6485'
HEAD='9ac4a59e96b1c69c8d617fbde9e98fe8c0436fed'; PARENT='73769114f081c8455aa21a393d0b992bd1283b67'; BASE='f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464'; BRANCH='codex/preserve-numeric-memo'
RECEIPT=A/'publish-declaration-ready-receipt.json'; assert not RECEIPT.exists(),'Reconcile receipt; never replay'
r={'started_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
def save(): RECEIPT.write_text(json.dumps(r,indent=2)+'\n')
def cmd(args):
 p=subprocess.run(args,cwd=W,capture_output=True,text=True)
 if p.returncode: raise RuntimeError('Command failed; no repeat mutation: '+p.stderr[:900])
 return p.stdout

def api(*args):
 v=json.loads(cmd(['/opt/homebrew/bin/gh','api',*args]))
 if isinstance(v,dict) and v.get('errors'): raise RuntimeError('GraphQL error; reconcile state without retry: '+str(v['errors'])[:600])
 return v

def fresh(expected,body=None):
 main=api('repos/sparq-org/sparq/git/ref/heads/main'); assert main['object']['sha']==BASE
 issue=api('repos/sparq-org/sparq/issues/6485'); assert issue['state']=='open' and not issue['assignees']
 pr=api('repos/sparq-org/sparq/pulls/6486'); assert pr['state']=='open' and pr['draft'] and pr['head']['sha']==expected and pr['head']['ref']==BRANCH and pr['base']['ref']=='main'
 assert not any(x['name'] in ['needs:user','needs:maintainer','hold','blocked','review:needs','review:changes','review:needs-owner'] for x in issue['labels']+pr['labels'])
 if body is not None: assert pr['body']==body,'Body changed; do not overwrite unknown edits'
 return {'main':main,'issue':issue,'pr':pr}
try:
 b=api('rate_limit')['resources']; assert min(b[k]['remaining'] for k in ['core','graphql'])>=100; r['budget']=b
 review=json.loads((A/'opus-memo-declaration-result.json').read_text()); assert review['verdict']=='APPROVE_FOR_CI' and review['reviewed_head']==HEAD and not review['blocking_findings']
 decision=json.loads((A/'opus-memo-declaration-root-disposition.json').read_text()); assert decision['head']==HEAD and decision['publish_and_ready_approved']
 assert cmd(['git','rev-parse','HEAD']).strip()==HEAD and cmd(['git','rev-parse','HEAD^']).strip()==PARENT and not cmd(['git','status','--porcelain'])
 assert cmd(['git','diff','--name-only',PARENT,HEAD]).splitlines()==['bench/feature-off-declarations/6486.json']
 assert cmd(['git','show',HEAD+':crates/sparq-core/src/lib.rs'])==cmd(['git','show',PARENT+':crates/sparq-core/src/lib.rs'])
 reg=api('repos/jeswr/agent-account-registry/actions/permissions'); assert reg['enabled'] is False; r['registry']=reg
 r['ownership']={}
 for n in [4097,4354,4247]:
  p=api('repos/sparq-org/sparq/pulls/'+str(n)); old=json.loads((A/('pr'+str(n)+'-current.json')).read_text()); assert p['state']=='open' and p['head']['sha']==old['head']['sha']; r['ownership'][str(n)]=p
 r['prepush']=fresh(PARENT,(A/'pr-draft-body.md').read_text()); remote=cmd(['git','ls-remote','--heads','origin','refs/heads/'+BRANCH]).split(); assert remote and remote[0]==PARENT; save()
 r['push_attempted']=True; save(); r['push_output']=cmd(['git','push','origin',HEAD+':refs/heads/'+BRANCH]); r['push_completed']=True; save()
 assert api('repos/sparq-org/sparq/git/ref/heads/'+BRANCH)['object']['sha']==HEAD
 r['prebody']=fresh(HEAD,(A/'pr-draft-body.md').read_text()); save()
 r['body_update']=api('repos/sparq-org/sparq/pulls/6486','-X','PATCH','--input',str(A/'pr-ready-body-input.json')); save()
 r['preready']=fresh(HEAD,(A/'pr-ready-body.md').read_text()); save(); node=r['preready']['pr']['node_id']
 q={'query':'mutation($id:ID!){markPullRequestReadyForReview(input:{pullRequestId:$id}){pullRequest{id isDraft headRefOid}}}','variables':{'id':node}}
 (A/'ready6486-input.json').write_text(json.dumps(q)+'\n'); r['ready_attempted']=True; save()
 r['ready']=api('graphql','--input',str(A/'ready6486-input.json')); save(); state=r['ready']['data']['markPullRequestReadyForReview']['pullRequest']; assert state['headRefOid']==HEAD and not state['isDraft']
 final=api('repos/sparq-org/sparq/pulls/6486'); assert final['state']=='open' and not final['draft'] and final['head']['sha']==HEAD
 r['verification']=final; r['completed_at']=datetime.datetime.now(datetime.timezone.utc).isoformat(); save(); print(json.dumps({'pr':6486,'url':final['html_url'],'head':HEAD,'draft':final['draft'],'state':final['state']}))
except Exception as e:
 r['error']=str(e); save(); raise
