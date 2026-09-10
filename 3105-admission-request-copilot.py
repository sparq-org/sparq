import subprocess,json,pathlib,datetime
a=pathlib.Path(__file__).parent
def run(args):
 p=subprocess.run(['/opt/homebrew/bin/gh',*args],capture_output=True,text=True)
 if p.returncode: raise SystemExit('GitHub operation failed without retry: '+p.stderr[:300])
 return json.loads(p.stdout)
r=run(['pr','view','6477','--repo','sparq-org/sparq','--json','state,isDraft,headRefOid,reviewRequests,reviews']);(a/'copilot-precheck.json').write_text(json.dumps(r,indent=2)+'\n');assert r['state']=='OPEN' and not r['isDraft'] and r['headRefOid']=='19763bfab1dce196a654c899b172e7b24d70bc59'
if any('copilot' in str(x).lower() for x in r['reviewRequests']+r['reviews']): print('Copilot already requested/reviewed; no duplicate request.')
else:
 payload=a/'copilot-request.json';payload.write_text(json.dumps({'reviewers':['copilot-pull-request-reviewer[bot]']})+'\n');out=run(['api','--method','POST','repos/sparq-org/sparq/pulls/6477/requested_reviewers','--input',str(payload)]);assert out['number']==6477 and out['head']['sha']==r['headRefOid']; receipt={'at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'accepted':True,'number':out['number'],'head':out['head']['sha'],'state':out['state'],'requested_reviewers':out.get('requested_reviewers',[])};(a/'copilot-request-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt))
