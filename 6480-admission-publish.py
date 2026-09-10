import datetime
import json
from pathlib import Path
import subprocess

ROOT = Path('/private/tmp/sparq-pr6049/.throughput-monitor')
OUT = ROOT / 'direct-6480/admission'
WORKTREE = ROOT / 'worktrees/issue6480'
HEAD = '26d139520f07f1ceacafbacbeb9991de371e2b53'
BASE = '781f667c19a8ebb779cfccb24b05ea432360b025'
BRANCH = 'codex/next-security-patch'
receipt = {'started_at': datetime.datetime.now(datetime.timezone.utc).isoformat()}

def save():
    (OUT / 'publication-receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')

def command(args):
    result = subprocess.run(args, cwd=WORKTREE, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(f'Command failed ({result.returncode}): {result.stderr}')
    return result.stdout

def api(*args):
    raw = command(['/opt/homebrew/bin/gh', 'api', *args])
    return json.loads(raw) if raw.strip() else None

def fresh_target():
    main = api('repos/sparq-org/sparq/git/ref/heads/main')
    issue = api('repos/sparq-org/sparq/issues/6480')
    prs = api('repos/sparq-org/sparq/pulls?head=sparq-org:codex/next-security-patch&state=all&per_page=100')
    assert main['object']['sha'] == BASE
    assert issue['state'] == 'open' and issue['number'] == 6480
    assert not prs, 'Existing PR must be reconciled before any duplicate mutation'
    return {'main': main, 'issue': issue, 'prs': prs}

try:
    budget = api('rate_limit')
    assert budget['resources']['core']['remaining'] >= 30, 'Fail closed: insufficient request reserve'
    receipt['budget'] = budget['resources']['core']
    for name, verdict, head in [
        ('opus-patch-result.json', 'approve_for_ci', '50c09eb2b7563924cccde9305ee51ff62b27af38'),
        ('opus-ci-proof-result.json', 'approve_delta_for_ci', HEAD),
    ]:
        review = json.loads((OUT / name).read_text())
        assert review['verdict'] == verdict and review['reviewed_head'] == head
        assert not review['blocking_findings']
    assert command(['git', 'rev-parse', 'HEAD']).strip() == HEAD
    assert command(['git', 'branch', '--show-current']).strip() == BRANCH
    assert not command(['git', 'status', '--porcelain'])
    assert command(['git', 'diff', '--name-only', BASE, HEAD]).splitlines() == [
        '.github/workflows/js.yml', 'gui/app/package.json', 'package-lock.json', 'site/package.json'
    ]
    registry = api('repos/jeswr/agent-account-registry/actions/permissions')
    assert registry['enabled'] is False
    receipt['registry'] = registry
    receipt['prepush'] = fresh_target()
    remote = command(['git', 'ls-remote', '--heads', 'origin', 'refs/heads/' + BRANCH]).strip()
    assert not remote or remote.split()[0] == HEAD, 'Unexpected existing branch head'
    save()
    if not remote:
        push = subprocess.run(['git', 'push', 'origin', HEAD + ':refs/heads/' + BRANCH], cwd=WORKTREE, text=True, capture_output=True)
        receipt['push'] = {'exit_code': push.returncode, 'stdout': push.stdout, 'stderr': push.stderr}
        save()
        assert push.returncode == 0, 'Push failed; inspect receipt before any further action'
    else:
        receipt['push'] = {'state': 'already_published_exact_head; no duplicate push'}
        save()
    ref = api('repos/sparq-org/sparq/git/ref/heads/' + BRANCH)
    assert ref['object']['sha'] == HEAD, 'Remote visibility must be reconciled without repeating push'
    receipt['precreate'] = fresh_target()
    save()
    created = api('repos/sparq-org/sparq/pulls', '-X', 'POST', '--input', str(OUT / 'pr-create-input.json'))
    receipt['created'] = created
    save()
    assert created['state'] == 'open' and created['head']['sha'] == HEAD and created['base']['ref'] == 'main'
    receipt['completed_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    save()
    print(json.dumps({'number': created['number'], 'url': created['html_url'], 'head': created['head']['sha'], 'draft': created['draft']}))
except Exception as error:
    receipt['error'] = str(error)
    save()
    raise
