from pathlib import Path
import datetime
import json
import subprocess

A = Path(__file__).parent
HEAD = '0b4554b924a80432cc1b572bd19f8e58cdbfb4e6'
BASE = '202879e9b4104f4b5ae8d9e8e5206eb2ff454b7f'
RECEIPT = A / 'enqueue6487-receipt.json'
assert not RECEIPT.exists() and not (A / 'enqueue6487-input.json').exists(), 'Reconcile prior dispatch; never replay'
receipt = {'started_at': datetime.datetime.now(datetime.timezone.utc).isoformat()}

def save():
    RECEIPT.write_text(json.dumps(receipt, indent=2) + '\n')

def api(*args):
    p = subprocess.run(['/opt/homebrew/bin/gh', 'api', *args], text=True, capture_output=True)
    if p.returncode:
        raise RuntimeError('GitHub operation failed; no retry: ' + p.stderr[:500])
    d = json.loads(p.stdout)
    if isinstance(d, dict) and d.get('errors'):
        raise RuntimeError('GraphQL error; no retry: ' + str(d['errors'])[:500])
    return d

try:
    proof = json.loads((A / 'ci6487-root-verification.json').read_text())
    assert proof['head'] == HEAD and proof['all_required_validation_passed']
    review = json.loads((A / 'opus-duplicate-final-result.json').read_text())
    assert review['reviewed_head'] == HEAD and review['verdict'] == 'APPROVE_FOR_CI' and not review['blocking_findings']
    budget = api('rate_limit')['resources']
    assert min(budget[k]['remaining'] for k in ['core', 'graphql']) > 100
    receipt['budget'] = budget
    rules = api('repos/sparq-org/sparq/rules/branches/main')
    bytype = {v['type']: v for v in rules}
    assert bytype['required_status_checks']['parameters']['required_status_checks'] == [{'context': 'gate', 'integration_id': 15368}]
    assert bytype['merge_queue']['parameters']['grouping_strategy'] == 'ALLGREEN'
    assert bytype['pull_request']['parameters']['required_approving_review_count'] == 0
    assert bytype['pull_request']['parameters']['required_review_thread_resolution']
    receipt['rules'] = rules
    registry = api('repos/jeswr/agent-account-registry/actions/permissions')
    assert registry['enabled'] is False
    receipt['registry'] = registry
    receipt['main_runs'] = []
    for run_id in [34522399163, 34522399200, 34522398851]:
        run = api(f'repos/sparq-org/sparq/actions/runs/{run_id}')
        assert run['head_sha'] == BASE and run['status'] == 'completed' and run['conclusion'] == 'success'
        receipt['main_runs'].append(run)
    gate = api(f"repos/sparq-org/sparq/check-runs/{proof['gate_check_id']}")
    assert gate['head_sha'] == HEAD and gate['name'] == 'gate' and gate['app']['id'] == 15368
    assert gate['status'] == 'completed' and gate['conclusion'] == 'success'
    receipt['gate'] = gate
    d = api('graphql', '--input', str(A / 'target6487-query.json'))
    repo = d['data']['repository']
    r = repo['pullRequest']
    assert repo['defaultBranchRef']['target']['oid'] == BASE
    assert r['state'] == 'OPEN' and not r['isDraft'] and r['baseRefName'] == 'main' and r['headRefOid'] == HEAD
    assert r['mergeable'] == 'MERGEABLE' and r['mergeStateStatus'] == 'CLEAN' and r['mergeQueueEntry'] is None
    assert not r['reviews']['pageInfo']['hasPreviousPage'] and all(x['state'] != 'CHANGES_REQUESTED' for x in r['reviews']['nodes'])
    assert not r['reviewThreads']['pageInfo']['hasNextPage'] and all(x['isResolved'] for x in r['reviewThreads']['nodes'])
    assert not r['labels']['pageInfo']['hasNextPage']
    assert all(not x['name'].startswith('review:needs') and x['name'] not in ['do-not-merge', 'blocked', 'needs-owner', 'hold'] for x in r['labels']['nodes'])
    assert r['body'] == (A / 'pr-ready-body.md').read_text()
    commit = r['commits']['nodes'][0]['commit']
    assert commit['oid'] == HEAD and commit['statusCheckRollup']['state'] == 'SUCCESS'
    assert any(x['author'] and x['author']['login'] in ['Copilot','copilot-pull-request-reviewer'] and x['commit'] and x['commit']['oid'] == HEAD for x in r['reviews']['nodes']), 'Current Copilot review required'
    receipt['pre_enqueue'] = d
    save()
    payload = {'query': 'mutation($input:EnqueuePullRequestInput!){enqueuePullRequest(input:$input){mergeQueueEntry{id position state headCommit{oid}}}}', 'variables': {'input': {'pullRequestId': r['id'], 'expectedHeadOid': HEAD, 'jump': False}}}
    (A / 'enqueue6487-input.json').write_text(json.dumps(payload) + '\n')
    receipt['result'] = api('graphql', '--input', str(A / 'enqueue6487-input.json'))
    receipt['completed_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    save()
    print(json.dumps(receipt['result']))
except Exception as e:
    receipt['error'] = str(e)
    save()
    raise
