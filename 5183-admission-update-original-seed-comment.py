from pathlib import Path
import json,subprocess,hashlib,datetime
r=Path('/private/tmp/sparq-pr6049/.throughput-monitor'); a=r/'direct-5183/admission'
assert not (a/'original-seed-comment-receipt.json').exists()
def api(name,path,method='GET',input_name=None):
 args=['/opt/homebrew/bin/gh','api',path]
 if method!='GET':args+=['--method',method,'--input',str(a/input_name)]
 c=subprocess.run(args,capture_output=True);(a/('original-seed-comment-'+name+'.json')).write_bytes(c.stdout)
 if c.returncode:raise SystemExit('Request failed; stop without retry: '+name)
 return json.loads(c.stdout)
b=api('budget','rate_limit');assert b['resources']['core']['remaining']>20
i=api('issue','repos/sparq-org/sparq/issues/5183');assert i['state']=='open'
c=api('precheck','repos/sparq-org/sparq/issues/comments/5618039976');expected=json.loads((a/'original-seed-comment-expected.json').read_text());assert c['user']['login']=='jeswr' and hashlib.sha256(c['body'].encode()).hexdigest()==expected['body_sha256']
c=api('result','repos/sparq-org/sparq/issues/comments/5618039976','PATCH','original-seed-comment-input.json');assert c['body']==json.loads((a/'original-seed-comment-input.json').read_text())['body']
receipt={'at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'comment':c['html_url'],'issue_state':i['state'],'body_sha256':hashlib.sha256(c['body'].encode()).hexdigest()};(a/'original-seed-comment-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt))
