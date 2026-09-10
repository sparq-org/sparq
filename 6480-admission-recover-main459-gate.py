from pathlib import Path
import json,subprocess,datetime,hashlib
r=Path('/private/tmp/sparq-pr6049/.throughput-monitor');a=r/'direct-6480/admission';w=r/'worktrees/issue5183'; H='4595388de9e389f5369d63828fbf90cfc16b62d9'; RID=34475426945
receipt={'started_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'scope':'One failed aggregate-only rerun; no sibling cancellation/rerun, protection change, registry enablement or release','evidence':'main459-gate-failure/gate.log explicitly calls for rerun after long legs finish; fresh complete451 check inventory and45 workflow runs have no pending work'}
def save(): (a/'gate-recovery-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
def api(path,name,method='GET'):
 p=subprocess.run(['/opt/homebrew/bin/gh','api',path,'-X',method],capture_output=True,text=True)
 receipt[name]={'exit_code':p.returncode,'stderr':p.stderr};save()
 if p.returncode: raise RuntimeError('Request failed; no retry: '+name)
 j=json.loads(p.stdout) if p.stdout.strip() else None
 (a/('gate-recovery-'+name+'.json')).write_text(json.dumps(j,indent=2)+'\n');return j
assert not (a/'gate-recovery-receipt.json').exists(),'Existing recovery must be reconciled, never duplicated'
try:
 s=json.loads((a/'gate-recovery1400-checks-summary.json').read_text());assert s['total']==451
 assert (datetime.datetime.now(datetime.timezone.utc)-datetime.datetime.fromisoformat(s['at'])).total_seconds()<600
 reg=json.loads(subprocess.run(['git','-C',str(w),'show',H+':.github/advisory-registry.json'],capture_output=True,text=True,check=True).stdout)['jobs']
 for x in s['nonpassing']:
  assert x['status']=='completed'
  if x['id']==102864922275: assert x['name']=='gate' and x['conclusion']=='failure'
  else: assert x['name'] in reg
 receipt['advisory_nonpassing']=[x['name'] for x in s['nonpassing'] if x['id']!=102864922275]
 budget=api('rate_limit','budget');assert budget['resources']['core']['remaining']>=30
 main=api('repos/sparq-org/sparq/git/ref/heads/main','main');assert main['object']['sha']==H
 runs=api('repos/sparq-org/sparq/actions/runs?head_sha='+H+'&per_page=100','runs')
 assert len(runs['workflow_runs'])==runs['total_count']
 assert all(x['status']=='completed' for x in runs['workflow_runs'])
 byid={x['id']:x for x in runs['workflow_runs']}
 for i in [34475427098,34475427058]:
  x=byid[i];assert x['conclusion']=='success' and x['head_sha']==H and x['event']=='push'
 x=byid[RID];assert x['conclusion']=='failure' and x['run_attempt']==1 and x['head_sha']==H and x['event']=='push' and x['head_branch']=='main' and x['path']=='.github/workflows/ci-summary.yml'
 assert not [x for x in runs['workflow_runs'] if x['name']=='ci-summary' and x['event']=='push' and x['id']>RID]
 registry=api('repos/jeswr/agent-account-registry/actions/permissions','registry');assert registry['enabled'] is False
 run=api(f'repos/sparq-org/sparq/actions/runs/{RID}','immediate-run');assert run['status']=='completed' and run['conclusion']=='failure' and run['run_attempt']==1 and run['head_sha']==H
 receipt['mutation_started_at']=datetime.datetime.now(datetime.timezone.utc).isoformat();save()
 api(f'repos/sparq-org/sparq/actions/runs/{RID}/rerun-failed-jobs','single-rerun','POST')
 receipt['completed_at']=datetime.datetime.now(datetime.timezone.utc).isoformat();receipt['result']='single aggregate failed-job rerun accepted; wait for actual result';save();print(json.dumps({'run':RID,'result':receipt['result']}))
except Exception as e:
 receipt['error']=str(e);save();raise
