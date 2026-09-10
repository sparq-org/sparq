from pathlib import Path
import json,subprocess,datetime
r=Path('/private/tmp/sparq-pr6049/.throughput-monitor'); a=r/'direct-6480/admission'
assert not (a/'close6480-receipt.json').exists()
def api(name,path,method='GET',input_name=None):
 args=['/opt/homebrew/bin/gh','api',path]
 if method!='GET':args+=['--method',method,'--input',str(a/input_name)]
 c=subprocess.run(args,capture_output=True);(a/('close6480-'+name+'.json')).write_bytes(c.stdout)
 if c.returncode:raise SystemExit('GitHub request failed; stop without retry: '+name)
 return json.loads(c.stdout)
b=api('budget','rate_limit');assert b['resources']['core']['remaining']>40
p=api('pr','repos/sparq-org/sparq/pulls/6481');assert p['merged'] and p['merge_commit_sha']=='4595388de9e389f5369d63828fbf90cfc16b62d9' and p['head']['sha']=='17594d4a6534c142cae764772fc42049e898eca3'
for n in [32,106,107]:
 x=api('alert'+str(n),'repos/sparq-org/sparq/dependabot/alerts/'+str(n));assert x['state']=='fixed' and x['dismissed_at'] is None
reg=api('registry','repos/jeswr/agent-account-registry/actions/permissions');assert reg['enabled'] is False
c=api('comment-precheck','repos/sparq-org/sparq/issues/comments/5616304836');assert c['id']==5616304836 and c['user']['login']=='jeswr' and c['body'].startswith('> 🤖 **SPARQ agent**')
i=api('issue-precheck','repos/sparq-org/sparq/issues/6480');assert i['state']=='open'
c=api('comment-result','repos/sparq-org/sparq/issues/comments/5616304836','PATCH','close6480-comment-input.json')
i=api('issue-final-precheck','repos/sparq-org/sparq/issues/6480');assert i['state']=='open'
i=api('issue-result','repos/sparq-org/sparq/issues/6480','PATCH','close6480-issue-input.json');assert i['state']=='closed' and i['state_reason']=='completed'
receipt={'at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'issue':i['html_url'],'state':i['state'],'state_reason':i['state_reason'],'closed_at':i['closed_at'],'comment':c['html_url'],'registry_enabled':reg['enabled'],'alerts_fixed':[32,106,107]};(a/'close6480-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt))
