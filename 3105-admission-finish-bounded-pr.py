import json,subprocess,pathlib,datetime
a=pathlib.Path(__file__).parent;head='ed66ef0931fa19dd521fac433870c86a78687a30'
def call(args):
 p=subprocess.run(['/opt/homebrew/bin/gh',*args],capture_output=True,text=True)
 if p.returncode:raise SystemExit('GitHub operation failed; no retry: '+p.stderr[:300])
 return p.stdout
r=json.loads(call(['pr','view','6477','--repo','sparq-org/sparq','--json','state,headRefOid,labels,body,reviewRequests,reviews']));(a/'bounded-reconcile-precheck.json').write_text(json.dumps(r,indent=2)+'\n');assert r['state']=='OPEN' and r['headRefOid']==head;assert any(x['name']=='review:changes' for x in r['labels'])
body=(a/'pr-bounded-body.md').read_text()
if r['body']!=body:call(['pr','edit','6477','--repo','sparq-org/sparq','--body-file',str(a/'pr-bounded-body.md')])
r=json.loads(call(['pr','view','6477','--repo','sparq-org/sparq','--json','state,headRefOid,labels,body,reviewRequests,reviews,statusCheckRollup']));assert r['headRefOid']==head and r['body']==body;(a/'bounded-pr-verified.json').write_text(json.dumps(r,indent=2)+'\n')
out=json.loads(call(['api','--method','POST','repos/sparq-org/sparq/pulls/6477/comments/3974906548/replies','--input',str(a/'bounded-thread-reply.json')]));(a/'bounded-thread-reply-receipt.json').write_text(json.dumps({'at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'head':head,'id':out['id'],'url':out['html_url'],'body':out['body']},indent=2)+'\n');assert out['body']==json.loads((a/'bounded-thread-reply.json').read_text())['body']
# A new source head warrants a fresh reviewer pass, unless one is already requested/completed.
r=json.loads(call(['pr','view','6477','--repo','sparq-org/sparq','--json','state,headRefOid,reviewRequests,reviews']));assert r['state']=='OPEN' and r['headRefOid']==head;prior=any('copilot' in str(x).lower() for x in r['reviewRequests']) or any('copilot' in x.get('author',{}).get('login','') and x.get('commit',{}).get('oid')==head for x in r['reviews'])
if not prior:
 out2=json.loads(call(['api','--method','POST','repos/sparq-org/sparq/pulls/6477/requested_reviewers','--input',str(a/'copilot-request.json')]));assert out2['head']['sha']==head;(a/'bounded-copilot-request-receipt.json').write_text(json.dumps({'at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'accepted':True,'head':head,'number':out2['number']},indent=2)+'\n')
print(json.dumps({'pr':6477,'head':head,'description_verified':True,'reply_url':out['html_url'],'copilot_already_requested_or_reviewed':prior,'hold_preserved':True}))
