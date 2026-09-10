from pathlib import Path
import subprocess,json,datetime,urllib.parse
r=Path('/private/tmp/sparq-pr6049/.throughput-monitor');a=r/'direct-5183/admission';H='fe3284199db0831f353d2ae401b8c69472764904';receipt={'started_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
assert not (a/'duplicate-followup-issue-receipt.json').exists(),'Reconcile existing creation before any repeat'
def api(path,name,method='GET',inp=None):
 args=['/opt/homebrew/bin/gh','api',path,'-X',method]
 if inp:args+=['--input',str(inp)]
 p=subprocess.run(args,capture_output=True,text=True);receipt[name]={'exit':p.returncode,'stderr':p.stderr}
 (a/'duplicate-followup-issue-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
 if p.returncode:raise RuntimeError('Request failed, no retry: '+name)
 j=json.loads(p.stdout);(a/('duplicate-followup-'+name+'.json')).write_text(json.dumps(j,indent=2)+'\n');return j
try:
 b=api('rate_limit','fresh-budget');assert b['resources']['core']['remaining']>=20 and b['resources']['search']['remaining']>=1
 q=urllib.parse.urlencode({'q':'repo:sparq-org/sparq is:issue "duplicate redistribution"','per_page':50});s=api('search/issues?'+q,'fresh-dedupe');assert s['total_count']==0,'Existing issue requires reconciliation'
 p=api('repos/sparq-org/sparq/pulls/6482','fresh-pr');assert p['state']=='open' and p['head']['sha']==H
 c=api('repos/sparq-org/sparq/issues','created','POST',a/'duplicate-followup-issue-input.json');receipt['created']={'number':c['number'],'url':c['html_url']};receipt['completed_at']=datetime.datetime.now(datetime.timezone.utc).isoformat();(a/'duplicate-followup-issue-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt['created']))
except Exception as e:
 receipt['error']=str(e);(a/'duplicate-followup-issue-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');raise
