from pathlib import Path
import datetime
import json
import subprocess
import urllib.parse

A = Path(__file__).parent
RECEIPT = A / 'memo-followup-issue-receipt.json'
assert not RECEIPT.exists(), 'Reconcile prior issue creation; never replay'
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
    b = api('rate_limit')['resources']
    assert min(b[k]['remaining'] for k in ['core', 'graphql']) > 100
    receipt['budget'] = b
    q = 'repo:sparq-org/sparq is:issue "Preserve the numeric-safety memo"'
    search = api('search/issues?' + urllib.parse.urlencode({'q': q, 'per_page': 100}))
    assert not search.get('incomplete_results') and search['total_count'] == 0, 'Potential duplicate; inspect before creating'
    receipt['dedupe'] = search
    parent = api('repos/sparq-org/sparq/issues/3113')
    assert parent['state'] == 'open' and not parent.get('pull_request')
    assert not parent['assignees']
    assert all(x['name'] not in ['needs:user', 'needs:maintainer', 'blocked', 'hold'] for x in parent['labels'])
    receipt['parent'] = parent
    main = api('repos/sparq-org/sparq/git/ref/heads/main')
    assert main['object']['sha'] == 'f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464'
    receipt['main'] = main
    save()
    created = api('repos/sparq-org/sparq/issues', '-X', 'POST', '--input', str(A / 'memo-followup-issue-input.json'))
    receipt['created'] = created
    receipt['completed_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    save()
    print(json.dumps({'number': created['number'], 'url': created['html_url'], 'state': created['state']}))
except Exception as e:
    receipt['error'] = str(e)
    save()
    raise
