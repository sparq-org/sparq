from pathlib import Path
import datetime
import json
import subprocess

A = Path(__file__).parent
HEAD = 'fe3284199db0831f353d2ae401b8c69472764904'
THREAD = 'PRRT_kwDOSz3qKM6hHXbJ'
COMMENT = 3980261208
RECEIPT = A / 'finish-review6482-receipt.json'
assert not RECEIPT.exists(), 'Reconcile existing receipt; never replay a mutation'
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

def fresh():
    d = api('graphql', '--input', str(A / 'target6482-query.json'))
    r = d['data']['repository']['pullRequest']
    assert r['state'] == 'OPEN' and not r['isDraft']
    assert r['headRefOid'] == HEAD and r['baseRefName'] == 'main'
    assert r['mergeQueueEntry'] is None
    assert not r['reviews']['pageInfo']['hasPreviousPage']
    assert all(x['state'] != 'CHANGES_REQUESTED' for x in r['reviews']['nodes'])
    assert not r['reviewThreads']['pageInfo']['hasNextPage']
    assert r['commits']['nodes'][0]['commit']['statusCheckRollup']['state'] == 'SUCCESS'
    return d, r

try:
    proof = json.loads((A / 'ci6482-root-verification.json').read_text())
    assert proof['head'] == HEAD and proof['all_required_validation_passed']
    assert proof['copilot_current_test_passed_in_linux_ci']
    budget = api('rate_limit')['resources']
    assert min(budget[k]['remaining'] for k in ['core', 'graphql']) > 100
    receipt['budget'] = budget
    d, r = fresh()
    thread = next(t for t in r['reviewThreads']['nodes'] if t['id'] == THREAD)
    assert not thread['isResolved'] and not thread['isOutdated']
    assert not thread['comments']['pageInfo']['hasNextPage']
    expected = json.loads((A / 'target6482-1512.json').read_text())['data']['repository']['pullRequest']
    assert r['reviewThreads'] == expected['reviewThreads'], 'Review discussion changed; reassess before mutation'
    old = next(c for t in expected['reviewThreads']['nodes'] for c in t['comments']['nodes'] if c['databaseId'] == COMMENT)
    current = next(c for c in thread['comments']['nodes'] if c['databaseId'] == COMMENT)
    assert current == old and current['author']['login'] == 'jeswr'
    receipt['pre_reply_update'] = d
    save()
    receipt['reply_updated'] = api(f'repos/sparq-org/sparq/pulls/comments/{COMMENT}', '-X', 'PATCH', '--input', str(A / 'copilot6482-ci-reply-input.json'))
    save()
    d, r = fresh()
    thread = next(t for t in r['reviewThreads']['nodes'] if t['id'] == THREAD)
    assert not thread['isResolved']
    assert next(c for c in thread['comments']['nodes'] if c['databaseId'] == COMMENT)['body'] == json.loads((A / 'copilot6482-ci-reply-input.json').read_text())['body']
    receipt['pre_resolve'] = d
    save()
    payload = {'query': 'mutation($id:ID!){resolveReviewThread(input:{threadId:$id}){thread{id isResolved}}}', 'variables': {'id': THREAD}}
    (A / 'resolve6482-input.json').write_text(json.dumps(payload) + '\n')
    receipt['resolved'] = api('graphql', '--input', str(A / 'resolve6482-input.json'))
    save()
    assert receipt['resolved']['data']['resolveReviewThread']['thread']['isResolved']
    d, r = fresh()
    assert all(t['isResolved'] for t in r['reviewThreads']['nodes'])
    assert r['body'] == (A / 'pr-body.md').read_text()
    receipt['pre_body_update'] = d
    save()
    receipt['body_updated'] = api('repos/sparq-org/sparq/pulls/6482', '-X', 'PATCH', '--input', str(A / 'pr6482-ci-body-input.json'))
    assert receipt['body_updated']['head']['sha'] == HEAD
    receipt['completed_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    save()
    print(json.dumps({'head': HEAD, 'thread_resolved': THREAD, 'reply': receipt['reply_updated']['html_url'], 'body_updated': True}))
except Exception as e:
    receipt['error'] = str(e)
    save()
    raise
