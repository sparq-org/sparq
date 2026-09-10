from pathlib import Path
import subprocess,json,datetime
r=Path('/private/tmp/sparq-pr6049/.throughput-monitor');a=r/'direct-6483/admission';H='0b4554b924a80432cc1b572bd19f8e58cdbfb4e6'
created=json.loads((a/'publication-create-pr.json').read_text());N=created['number'];assert created['head']['sha']==H
assert not (a/'copilot-request-receipt.json').exists(),'Reconcile existing request, never duplicate'
receipt={'at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'pr':N,'head':H}
def api(path,name,method='GET',data=None):
 args=['/opt/homebrew/bin/gh','api',path,'-X',method]
 if data:args+=['--input',str(data)]
 p=subprocess.run(args,capture_output=True,text=True)
 (a/(name+'.stderr')).write_text(p.stderr)
 if p.returncode:raise RuntimeError('Request failed; no retry: '+name)
 j=json.loads(p.stdout) if p.stdout.strip() else None;(a/(name+'.json')).write_text(json.dumps(j,indent=2)+'\n');return j
try:
 b=api('rate_limit','copilot-budget');assert b['resources']['core']['remaining']>=100
 reviews=api(f'repos/sparq-org/sparq/pulls/{N}/reviews?per_page=100','copilot-existing-reviews');assert len(reviews)<100
 p=api(f'repos/sparq-org/sparq/pulls/{N}','copilot-immediate-pr');assert p['state']=='open' and p['head']['sha']==H and not p['draft']
 pending=any('copilot' in x['login'].lower() for x in p.get('requested_reviewers',[]));reviewed=any('copilot' in x['user']['login'].lower() and x['commit_id']==H for x in reviews)
 if pending or reviewed:receipt['result']='Copilot already requested or reviewed current head; no duplicate request'
 else:
  inp=a/'copilot-request-input.json';inp.write_text(json.dumps({'reviewers':['copilot-pull-request-reviewer[bot]']})+'\n')
  out=api(f'repos/sparq-org/sparq/pulls/{N}/requested_reviewers','copilot-request-result','POST',inp)
  receipt['result']='Copilot review requested';receipt['requested_reviewers']=out.get('requested_reviewers',[])
 (a/'copilot-request-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps({'pr':N,'result':receipt['result']}))
except Exception as e:
 receipt['error']=str(e);(a/'copilot-request-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');raise
