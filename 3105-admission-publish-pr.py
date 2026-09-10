import json,pathlib,subprocess,datetime,re
a=pathlib.Path(__file__).parent
w=a.parents[1]/'worktrees/issue3105'
head='19763bfab1dce196a654c899b172e7b24d70bc59'
base='e53464c73f31f7aca800f3867ac054c36408e346'
branch='codex/capped-rhs-reuse'
log={'at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'head':head,'calls':[]}
def cmd(args,ok=(0,)):
 p=subprocess.run(args,cwd=w,capture_output=True,text=True)
 log['calls'].append({'args':args,'exit':p.returncode,'stdout':p.stdout,'stderr':p.stderr})
 (a/'publication-receipt.json').write_text(json.dumps(log,indent=2)+'\n')
 if p.returncode not in ok: raise SystemExit('Operation failed; no retry. See publication-receipt.json')
 return p
def gh(*args): return json.loads(cmd(['/opt/homebrew/bin/gh',*args]).stdout)
assert cmd(['git','rev-parse','HEAD']).stdout.strip()==head
assert not cmd(['git','status','--porcelain']).stdout.strip(), 'Stop: worktree temporarily changed; no publication.'
r=gh('api','rate_limit'); assert all(r['resources'][k]['remaining']>=50 for k in ['core','graphql']), 'Stop: request reserve low.'
q='query { rateLimit { remaining resetAt } repository(owner:"sparq-org",name:"sparq") { defaultBranchRef { target { oid } } issue(number:3105) { state labels(first:40) { nodes { name } } } pullRequests(first:10,headRefName:"codex/capped-rhs-reuse",states:[OPEN]) { nodes { number headRefOid url } } } }'
r=gh('api','graphql','-f','query='+q); assert not r.get('errors'); repo=r['data']['repository']; assert repo['defaultBranchRef']['target']['oid']==base; assert repo['issue']['state']=='OPEN'; assert not repo['pullRequests']['nodes']; assert not any(x['name'] in ['needs:user','needs:maintainer','review:needs','review:changes'] for x in repo['issue']['labels']['nodes'])
remote=cmd(['git','ls-remote','--exit-code','origin','refs/heads/'+branch],ok=(0,2)); assert remote.returncode==2 or remote.stdout.split()[0]==head,'Unexpected remote head; stop.'
cmd(['git','push','origin',head+':refs/heads/'+branch])
# Revalidate the target immediately before creating its PR.
r=gh('api','graphql','-f','query='+q); assert not r.get('errors'); repo=r['data']['repository']; assert repo['defaultBranchRef']['target']['oid']==base and repo['issue']['state']=='OPEN' and not repo['pullRequests']['nodes']
url=cmd(['/opt/homebrew/bin/gh','pr','create','--repo','sparq-org/sparq','--base','main','--head',branch,'--title','perf(engine): reuse RHS scans across capped seed blocks','--body-file',str(a/'pr-body.md')]).stdout.strip(); assert re.fullmatch(r'https://github.com/sparq-org/sparq/pull/[0-9]+',url); (a/'pr-url.txt').write_text(url+'\n')
n=url.rsplit('/',1)[1]; result=gh('pr','view',n,'--repo','sparq-org/sparq','--json','number,url,state,isDraft,headRefOid,headRefName,baseRefName,body,labels,statusCheckRollup'); assert result['headRefOid']==head and result['state']=='OPEN' and not result['isDraft']; assert result['body']==(a/'pr-body.md').read_text();(a/'pr-created.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'url':url,'head':head,'state':result['state'],'draft':result['isDraft']}))
