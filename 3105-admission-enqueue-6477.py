import datetime
import json
import pathlib
import subprocess

p = pathlib.Path(__file__).parent
head = 'ed66ef0931fa19dd521fac433870c86a78687a30'
base = 'e53464c73f31f7aca800f3867ac054c36408e346'

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

query = '''query {rateLimit{remaining resetAt} repository(owner:"sparq-org",name:"sparq"){defaultBranchRef{target{oid}} pullRequest(number:6477){id state isDraft baseRefName headRefOid body mergeable mergeStateStatus mergeQueueEntry{position state} labels(first:100){nodes{name}} reviews(last:20){nodes{state submittedAt}} reviewThreads(first:100){pageInfo{hasNextPage} nodes{isResolved}} commits(last:1){nodes{commit{oid statusCheckRollup{state}}}}}}}'''

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
    assert all(v['state'] != 'CHANGES_REQUESTED' and v['submittedAt'] <= '2026-09-10T03:05:43Z' for v in r['reviews']['nodes'])
    commit = r['commits']['nodes'][0]['commit']
    assert commit['oid'] == head and commit['statusCheckRollup']['state'] == 'SUCCESS'
    return x, r

assert not (p / 'enqueue-6477-result.json').exists(), 'Do not replay an admission mutation.'
rules = api('repos/sparq-org/sparq/rules/branches/main')
(p / 'queue-rules-0329.json').write_text(json.dumps(rules, indent=2) + '\n')
by_type = {v['type']: v for v in rules}
assert by_type['required_status_checks']['parameters']['required_status_checks'] == [{'context': 'gate', 'integration_id': 15368}]
assert by_type['merge_queue']['parameters']['grouping_strategy'] == 'ALLGREEN'
assert by_type['pull_request']['parameters']['required_approving_review_count'] == 0
assert by_type['pull_request']['parameters']['required_review_thread_resolution']
x, r = fresh()
assert r['body'] == (p / 'pr-source-cleared-body.md').read_text()
call('pr', 'edit', '6477', '--repo', 'sparq-org/sparq', '--body-file', str(p / 'pr-ci-green-body.md'))
x, r = fresh()
assert r['body'] == (p / 'pr-ci-green-body.md').read_text()
(p / 'queue-precheck-0329.json').write_text(json.dumps(x, indent=2) + '\n')
request = {'query': 'mutation($input:EnqueuePullRequestInput!){enqueuePullRequest(input:$input){mergeQueueEntry{id position state headCommit{oid}}}}', 'variables': {'input': {'pullRequestId': r['id'], 'expectedHeadOid': head, 'jump': False}}}
(p / 'enqueue-6477-input.json').write_text(json.dumps(request) + '\n')
out = api('graphql', '--input', str(p / 'enqueue-6477-input.json'))
(p / 'enqueue-6477-result.json').write_text(json.dumps({'at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'expected_head': head, 'result': out}, indent=2) + '\n')
print(json.dumps(out))
