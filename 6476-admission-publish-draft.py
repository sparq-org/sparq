import json
import pathlib
import re
import subprocess
import datetime

a = pathlib.Path(__file__).parent
w = a.parents[1] / 'worktrees/issue6476'
head = 'bcba08207b03714a25ad7a2a4774370c7f6f6bfa'
base = 'd41ec9fcb796504d85f5a5247a5fdc9eb3e65de5'
branch = 'codex/nested-query-budget'
receipt = {'at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'head': head, 'calls': []}

def cmd(args, ok=(0,)):
    p = subprocess.run(args, cwd=w, capture_output=True, text=True)
    receipt['calls'].append({'args': args, 'exit': p.returncode, 'stdout': p.stdout, 'stderr': p.stderr})
    (a / 'draft-publication-receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    if p.returncode not in ok:
        raise SystemExit('Operation failed; no retry. See draft-publication-receipt.json')
    return p

def gh(*args):
    return json.loads(cmd(['/opt/homebrew/bin/gh', *args]).stdout)

review = json.loads((a / 'opus-final-result.json').read_text())
assert review['verdict'] == 'approve_for_ci' and review['reviewed_head'] == head and not review['blocking_findings']
assert cmd(['git', 'rev-parse', 'HEAD']).stdout.strip() == head
assert cmd(['git', 'branch', '--show-current']).stdout.strip() == branch
assert not cmd(['git', 'status', '--porcelain']).stdout.strip()
budget = gh('api', 'rate_limit')
assert all(budget['resources'][k]['remaining'] >= 50 for k in ['core', 'graphql'])
query = '''query {
  rateLimit { remaining resetAt }
  repository(owner:"sparq-org",name:"sparq") {
    isPrivate
    defaultBranchRef { target { oid } }
    issue(number:6476) {
      state
      labels(first:40) { nodes { name } }
      timelineItems(last:30,itemTypes:[CROSS_REFERENCED_EVENT]) {
        nodes { ... on CrossReferencedEvent { source { ... on PullRequest { number state url headRefName } } } }
      }
    }
    pullRequests(first:10,headRefName:"codex/nested-query-budget",states:[OPEN]) {
      nodes { number headRefOid url }
    }
  }
}'''

def fresh():
    out = gh('api', 'graphql', '-f', 'query=' + query)
    assert not out.get('errors')
    repo = out['data']['repository']
    assert not repo['isPrivate'] and repo['defaultBranchRef']['target']['oid'] == base
    issue = repo['issue']
    assert issue['state'] == 'OPEN' and not repo['pullRequests']['nodes']
    assert not any(x['name'] in ['needs:user','needs:maintainer','review:needs','review:changes'] for x in issue['labels']['nodes'])
    assert not any((x.get('source') or {}).get('state') == 'OPEN' for x in issue['timelineItems']['nodes']), 'Open cross-referenced PR requires inspection.'
    return repo

fresh()
remote = cmd(['git', 'ls-remote', '--exit-code', 'origin', 'refs/heads/' + branch], ok=(0,2))
assert remote.returncode == 2 or remote.stdout.split()[0] == head
assert not cmd(['git', 'status', '--porcelain']).stdout.strip()
cmd(['git', 'push', 'origin', head + ':refs/heads/' + branch])
fresh()
url = cmd(['/opt/homebrew/bin/gh','pr','create','--repo','sparq-org/sparq','--base','main','--head',branch,'--draft','--title','fix(engine): restore outer budgets after nested queries','--body-file',str(a / 'pr-draft-body.md')]).stdout.strip()
assert re.fullmatch(r'https://github.com/sparq-org/sparq/pull/[0-9]+', url)
(a / 'pr-url.txt').write_text(url + '\n')
number = url.rsplit('/',1)[1]
result = gh('pr','view',number,'--repo','sparq-org/sparq','--json','number,url,state,isDraft,headRefOid,headRefName,baseRefName,body,labels,statusCheckRollup')
assert result['state'] == 'OPEN' and result['isDraft'] and result['headRefOid'] == head
assert result['body'] == (a / 'pr-draft-body.md').read_text()
(a / 'pr-created.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({'url': url, 'head': head, 'draft': True}))
