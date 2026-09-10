import json,subprocess,datetime,pathlib
a=pathlib.Path(__file__).parent
def gh(*args):
 p=subprocess.run(['/opt/homebrew/bin/gh',*args],capture_output=True,text=True)
 if p.returncode: raise SystemExit('GitHub call failed; no retry: '+p.stderr[:350])
 return json.loads(p.stdout)
r={'at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
r['rate_limit']=gh('api','rate_limit')
if min(r['rate_limit']['resources'][k]['remaining'] for k in ['core','graphql'])<50:
 (a/'health-final.json').write_text(json.dumps(r,indent=2)+'\n'); raise SystemExit('Budget reserve low; stop without further requests.')
query="""query { rateLimit { remaining resetAt } repository(owner:"sparq-org",name:"sparq") { defaultBranchRef { target { oid } } issue(number:3105) { id title state body labels(first:30) { nodes { name } } comments(last:6) { nodes { author { login } body createdAt } } } pullRequests(first:10,headRefName:"codex/capped-rhs-reuse",states:[OPEN]) { nodes { number state title headRefOid url } } pullRequest(number:6095) { state headRefOid labels(first:30) { nodes { name } } mergeQueueEntry { position } } } }"""
r['graphql']=gh('api','graphql','-f','query='+query)
if r['graphql'].get('errors'): raise SystemExit('GraphQL error; no retries.')
r['registry_permissions']=gh('api','repos/jeswr/agent-account-registry/actions/permissions')
r['main_runs']=gh('api','repos/sparq-org/sparq/actions/runs?branch=main&per_page=15')
(a/'health-final.json').write_text(json.dumps(r,indent=2)+'\n')
g=r['graphql']['data']; print(json.dumps({'at':r['at'],'budget':g['rateLimit'],'main':g['repository']['defaultBranchRef']['target']['oid'],'issue':{k:v for k,v in g['repository']['issue'].items() if k not in ['body','comments']},'same_branch_pr':g['repository']['pullRequests']['nodes'],'held6095':g['repository']['pullRequest'],'registry_permissions':r['registry_permissions'],'runs':[{'id':x['id'],'name':x['name'],'status':x['status'],'conclusion':x['conclusion'],'head_sha':x['head_sha'],'event':x['event']} for x in r['main_runs']['workflow_runs']]},indent=2))
