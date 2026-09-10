import datetime
import json
import pathlib
import subprocess

p = pathlib.Path(__file__).parent
head = '3c637b228449e9345d8bba31ea2d7d449f8a2b8f'
base = 'd41ec9fcb796504d85f5a5247a5fdc9eb3e65de5'

def call(*args):
    r = subprocess.run(['/opt/homebrew/bin/gh', *args], text=True, capture_output=True)
    if r.returncode:
        raise SystemExit('GitHub operation failed; no retry: ' + r.stderr[:300])
    return r.stdout

def api(*args):
    x = json.loads(call('api', *args))
    if isinstance(x, dict) and x.get('errors'):
        raise SystemExit('GraphQL error; no retry: ' + str(x['errors'])[:300])
    return x

proof = json.loads((p / 'head3c-final-ci-verification.json').read_text())
assert proof['head'] == head and proof['all_required_validation_passed']
budget = api('rate_limit')['resources']
assert min(budget[x]['remaining'] for x in ['core', 'graphql']) > 100
for filename, reviewed_head in [('opus-final-result.json', 'bcba08207b03714a25ad7a2a4774370c7f6f6bfa'), ('opus-followup-result.json', 'd07ca79f89e3a8945be46b516c3cd2f770ccd618'), ('opus-copilot-result.json', head)]:
    review = json.loads((p / filename).read_text())
    assert review['reviewed_head'] == reviewed_head and not review['blocking_findings']
    assert review['verdict'] in ['approve_for_ci', 'approve_delta_for_ci']

query = '''query {rateLimit{remaining resetAt} repository(owner:"sparq-org",name:"sparq"){defaultBranchRef{target{oid}} pullRequest(number:6478){id state isDraft baseRefName headRefOid body mergeable mergeStateStatus mergeQueueEntry{position state} labels(first:100){nodes{name}} reviews(last:20){nodes{state submittedAt}} reviewThreads(first:100){pageInfo{hasNextPage} nodes{isResolved}} commits(last:1){nodes{commit{oid statusCheckRollup{state}}}}}}}'''

def fresh():
    x = api('graphql', '-f', 'query=' + query)
    assert x['data']['rateLimit']['remaining'] > 100
    repo = x['data']['repository']
    assert repo['defaultBranchRef']['target']['oid'] == base
    r = repo['pullRequest']
    assert r['state'] == 'OPEN' and not r['isDraft'] and r['baseRefName'] == 'main'
    assert r['headRefOid'] == head and r['mergeable'] == 'MERGEABLE'
    assert r['mergeStateStatus'] == 'CLEAN' and r['mergeQueueEntry'] is None
    assert {v['name'] for v in r['labels']['nodes']} <= {'area:bench', 'area:sparq-engine', 'review:unreviewed'}
    assert not r['reviewThreads']['pageInfo']['hasNextPage']
    assert all(v['isResolved'] for v in r['reviewThreads']['nodes'])
    assert all(v['state'] != 'CHANGES_REQUESTED' and v['submittedAt'] <= '2026-09-10T07:25:43Z' for v in r['reviews']['nodes'])
    commit = r['commits']['nodes'][0]['commit']
    assert commit['oid'] == head and commit['statusCheckRollup']['state'] == 'SUCCESS'
    return x, r

assert not (p / 'enqueue-6478-result.json').exists(), 'Do not replay an admission mutation.'
rules = api('repos/sparq-org/sparq/rules/branches/main')
(p / 'queue-rules-0757.json').write_text(json.dumps(rules, indent=2) + '\n')
by_type = {v['type']: v for v in rules}
assert by_type['required_status_checks']['parameters']['required_status_checks'] == [{'context': 'gate', 'integration_id': 15368}]
assert by_type['merge_queue']['parameters']['grouping_strategy'] == 'ALLGREEN'
assert by_type['pull_request']['parameters']['required_approving_review_count'] == 0
assert by_type['pull_request']['parameters']['required_review_thread_resolution']
x, r = fresh()
assert r['body'] == (p / 'pr-copilot-followup-body.md').read_text()
call('pr', 'edit', '6478', '--repo', 'sparq-org/sparq', '--body-file', str(p / 'pr-head3c-ci-green-body.md'))
x, r = fresh()
assert r['body'] == (p / 'pr-head3c-ci-green-body.md').read_text()
(p / 'queue-precheck-0757.json').write_text(json.dumps(x, indent=2) + '\n')
request = {'query': 'mutation($input:EnqueuePullRequestInput!){enqueuePullRequest(input:$input){mergeQueueEntry{id position state headCommit{oid}}}}', 'variables': {'input': {'pullRequestId': r['id'], 'expectedHeadOid': head, 'jump': False}}}
(p / 'enqueue-6478-input.json').write_text(json.dumps(request) + '\n')
out = api('graphql', '--input', str(p / 'enqueue-6478-input.json'))
(p / 'enqueue-6478-result.json').write_text(json.dumps({'at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'expected_head': head, 'result': out}, indent=2) + '\n')
print(json.dumps(out))
