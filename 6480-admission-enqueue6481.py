from pathlib import Path
import json,datetime,subprocess
A=Path(__file__).parent;HEAD='17594d4a6534c142cae764772fc42049e898eca3';BASE='781f667c19a8ebb779cfccb24b05ea432360b025'
def api(*args):
 p=subprocess.run(['/opt/homebrew/bin/gh','api',*args],text=True,capture_output=True)
 if p.returncode:raise RuntimeError('GitHub operation failed; no retry: '+p.stderr[:500])
 d=json.loads(p.stdout)
 if isinstance(d,dict) and d.get('errors'):raise RuntimeError('GraphQL error; no retry: '+str(d['errors'])[:500])
 return d
assert not (A/'enqueue6481-result.json').exists(),'Do not replay a queue mutation'
assert not (A/'enqueue6481-input.json').exists(),'Reconcile any prior uncertain dispatch before retry'
proof=json.loads((A/'head175-final-ci-verification.json').read_text());assert proof['head']==HEAD and proof['all_required_validation_passed']
for name,head in [('opus-patch-result.json','50c09eb2b7563924cccde9305ee51ff62b27af38'),('opus-ci-proof-result.json','26d139520f07f1ceacafbacbeb9991de371e2b53'),('opus-followup-result.json','0c6a780593a9b3381fb158e426519a2a6d8d17f9'),('opus-native-smoke-result.json',HEAD)]:
 r=json.loads((A/name).read_text());assert r['reviewed_head']==head and r['verdict'] in ['approve_for_ci','approve_delta_for_ci'] and not r['blocking_findings']
budget=api('rate_limit')['resources'];assert min(budget[k]['remaining'] for k in ['core','graphql'])>100
rules=api('repos/sparq-org/sparq/rules/branches/main');bytype={v['type']:v for v in rules};assert bytype['required_status_checks']['parameters']['required_status_checks']==[{'context':'gate','integration_id':15368}];assert bytype['merge_queue']['parameters']['grouping_strategy']=='ALLGREEN';assert bytype['pull_request']['parameters']['required_approving_review_count']==0 and bytype['pull_request']['parameters']['required_review_thread_resolution']
registry=api('repos/jeswr/agent-account-registry/actions/permissions');assert registry['enabled'] is False
gate=api('repos/sparq-org/sparq/check-runs/102837808339');assert gate['head_sha']==HEAD and gate['app']['id']==15368 and gate['name']=='gate' and gate['conclusion']=='success' and gate['status']=='completed'
d=api('graphql','--input',str(A/'final175-target-query.json'));repo=d['data']['repository'];r=repo['pullRequest'];assert repo['defaultBranchRef']['target']['oid']==BASE;assert r['state']=='OPEN' and not r['isDraft'] and r['baseRefName']=='main' and r['headRefOid']==HEAD;assert r['mergeable']=='MERGEABLE' and r['mergeStateStatus']=='CLEAN' and r['mergeQueueEntry'] is None;assert not r['reviewThreads']['pageInfo']['hasNextPage'] and all(x['isResolved'] for x in r['reviewThreads']['nodes']);assert len(r['reviews']['nodes'])<40 and all(x['state']!='CHANGES_REQUESTED' for x in r['reviews']['nodes']);assert all(not x['name'].startswith('review:needs') and x['name'] not in ['do-not-merge','blocked','needs-owner','hold'] for x in r['labels']['nodes']);assert r['body']==(A/'pr175-ci-green-body.md').read_text();commit=r['commits']['nodes'][0]['commit'];assert commit['oid']==HEAD and commit['statusCheckRollup']['state']=='SUCCESS'
receipt={'at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'budget':budget,'rules':rules,'registry':registry,'gate':gate,'target':d};(A/'enqueue6481-precheck.json').write_text(json.dumps(receipt,indent=2)+'\n')
payload={'query':'mutation($input:EnqueuePullRequestInput!){enqueuePullRequest(input:$input){mergeQueueEntry{id position state headCommit{oid}}}}','variables':{'input':{'pullRequestId':r['id'],'expectedHeadOid':HEAD,'jump':False}}};(A/'enqueue6481-input.json').write_text(json.dumps(payload)+'\n')
result=api('graphql','--input',str(A/'enqueue6481-input.json'));(A/'enqueue6481-result.json').write_text(json.dumps({'at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'expected_head':HEAD,'result':result},indent=2)+'\n');print(json.dumps(result))
