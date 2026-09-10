import datetime
import json
import pathlib
import subprocess

a = pathlib.Path(__file__).parent
w = a.parents[1] / 'worktrees/issue6476'
old = 'bcba08207b03714a25ad7a2a4774370c7f6f6bfa'
new = 'd07ca79f89e3a8945be46b516c3cd2f770ccd618'
base = 'd41ec9fcb796504d85f5a5247a5fdc9eb3e65de5'
branch = 'codex/nested-query-budget'
receipt = {'at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'old': old, 'new': new, 'calls': []}

def cmd(args):
    p = subprocess.run(args, cwd=w, capture_output=True, text=True)
    receipt['calls'].append({'args': args, 'exit': p.returncode, 'stdout': p.stdout, 'stderr': p.stderr})
    (a / 'ready-followup-receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    if p.returncode:
        raise SystemExit('Operation failed; no blind retry. See ready-followup-receipt.json')
    return p.stdout

def gh(*args):
    return json.loads(cmd(['/opt/homebrew/bin/gh', *args]))

review = json.loads((a / 'opus-followup-result.json').read_text())
assert review['verdict'] == 'approve_delta_for_ci' and review['reviewed_head'] == new
assert not review['blocking_findings'] and review['v2_declaration_review']['approved']
assert cmd(['git','rev-parse','HEAD']).strip() == new and not cmd(['git','status','--porcelain']).strip()
cmd(['git','merge-base','--is-ancestor',old,new])
budget = gh('api','rate_limit')
assert all(budget['resources'][k]['remaining'] >= 50 for k in ['core','graphql'])
query = '''query { rateLimit { remaining resetAt }
  repository(owner:"sparq-org",name:"sparq") {
    defaultBranchRef { target { oid } }
    pullRequest(number:6478) { id state isDraft headRefOid headRefName baseRefName
      mergeQueueEntry { position }
      labels(first:40) { nodes { name } }
    }
  }
}'''

def fresh(expected, draft):
    out = gh('api','graphql','-f','query=' + query)
    assert not out.get('errors')
    repo = out['data']['repository']
    assert repo['defaultBranchRef']['target']['oid'] == base
    pr = repo['pullRequest']
    assert pr['state'] == 'OPEN' and pr['headRefOid'] == expected and pr['isDraft'] == draft
    assert pr['headRefName'] == branch and pr['baseRefName'] == 'main' and pr['mergeQueueEntry'] is None
    assert not any(x['name'] in ['needs:user','needs:maintainer','review:needs-user'] for x in pr['labels']['nodes'])
    return pr

fresh(new, True)
cmd(['/opt/homebrew/bin/gh','pr','edit','6478','--repo','sparq-org/sparq','--body-file',str(a / 'pr-ci-body.md')])
fresh(new, True)
cmd(['/opt/homebrew/bin/gh','pr','ready','6478','--repo','sparq-org/sparq'])
fresh(new, False)
pr = gh('api','repos/sparq-org/sparq/pulls/6478')
assert pr['head']['sha'] == new and pr['state'] == 'open' and not pr['draft']
assert pr['body'] == (a / 'pr-ci-body.md').read_text()
reviews = gh('api','repos/sparq-org/sparq/pulls/6478/reviews?per_page=100')
pending = any('copilot' in x['login'].lower() for x in pr.get('requested_reviewers', []))
reviewed = any('copilot' in x['user']['login'].lower() and x['commit_id'] == new for x in reviews)
if not pending and not reviewed:
    fresh(new, False)
    (a / 'copilot-request.json').write_text(json.dumps({'reviewers': ['copilot-pull-request-reviewer[bot]']}) + '\n')
    out = gh('api','--method','POST','repos/sparq-org/sparq/pulls/6478/requested_reviewers','--input',str(a / 'copilot-request.json'))
    assert out['number'] == 6478 and out['head']['sha'] == new
    (a / 'copilot-request-receipt.json').write_text(json.dumps({'at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'head': new, 'requested_reviewers': out.get('requested_reviewers', [])}, indent=2) + '\n')
result = gh('pr','view','6478','--repo','sparq-org/sparq','--json','number,url,state,isDraft,headRefOid,labels,reviewRequests,reviews,statusCheckRollup')
assert result['headRefOid'] == new and result['state'] == 'OPEN' and not result['isDraft']
(a / 'pr-ready-verified.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({'pr': 6478, 'head': new, 'draft': False, 'copilot_already_pending_or_reviewed': pending or reviewed}))
