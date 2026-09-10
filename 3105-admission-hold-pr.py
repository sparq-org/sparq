import pathlib,subprocess,json,datetime
a=pathlib.Path(__file__).parent
def cmd(args):
 p=subprocess.run(['/opt/homebrew/bin/gh',*args],capture_output=True,text=True)
 if p.returncode: raise SystemExit('GitHub operation failed, no retry: '+p.stderr[:300])
 return p.stdout
q='query { repository(owner:"sparq-org",name:"sparq") { pullRequest(number:6477) { state headRefOid mergeQueueEntry { position } labels(first:40) { nodes { name } } } } }'
r=json.loads(cmd(['api','graphql','-f','query='+q])); (a/'memory-hold-precheck.json').write_text(json.dumps(r,indent=2)+'\n');p=r['data']['repository']['pullRequest'];assert p['state']=='OPEN' and p['headRefOid']=='19763bfab1dce196a654c899b172e7b24d70bc59'; assert p['mergeQueueEntry'] is None
if not any(x['name']=='review:changes' for x in p['labels']['nodes']):
 payload=a/'memory-hold-label.json';payload.write_text(json.dumps({'labels':['review:changes']})+'\n');d=json.loads(cmd(['api','--method','POST','repos/sparq-org/sparq/issues/6477/labels','--input',str(payload)]));assert any(x['name']=='review:changes' for x in d)
url=cmd(['pr','comment','6477','--repo','sparq-org/sparq','--body-file',str(a/'memory-hold-comment.md')]).strip();(a/'memory-hold-result.json').write_text(json.dumps({'at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'head':p['headRefOid'],'label':'review:changes','comment_url':url},indent=2)+'\n');print(url)
