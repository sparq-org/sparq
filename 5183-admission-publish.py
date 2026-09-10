import datetime
import json
from pathlib import Path
import subprocess

ROOT = Path('/private/tmp/sparq-pr6049/.throughput-monitor')
OUT = ROOT / 'direct-5183/admission'
WORKTREE = ROOT / 'worktrees/issue5183'
HEAD = 'fe3284199db0831f353d2ae401b8c69472764904'
BASE = '4595388de9e389f5369d63828fbf90cfc16b62d9'
BRANCH = 'codex/update-differential-replay'
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
    issue = api('repos/sparq-org/sparq/issues/5183')
    prs = api('repos/sparq-org/sparq/pulls?head=sparq-org:codex/update-differential-replay&state=all&per_page=100')
    assert main['object']['sha'] == BASE
    assert issue['state'] == 'open' and issue['number'] == 5183
    assert not prs, 'Existing PR must be reconciled before any duplicate mutation'
    return {'main': main, 'issue': issue, 'prs': prs}

assert not (OUT / 'publication-receipt.json').exists(), 'Prior publication must be reconciled before any repeat'

try:
    budget = api('rate_limit')
    assert budget['resources']['core']['remaining'] >= 30, 'Fail closed: insufficient request reserve'
    receipt['budget'] = budget['resources']['core']
    review = json.loads((OUT / 'opus-edition-followup-result.json').read_text())
    assert review['verdict'] == 'APPROVE_FOR_CI' and review['reviewed_head'] == HEAD
    assert not review['blocking_findings']
    disposition = json.loads((OUT / 'opus-edition-followup-root-disposition.json').read_text())
    assert disposition['publish_approved'] is True and disposition['head'] == HEAD
    assert command(['git', 'rev-parse', 'HEAD']).strip() == HEAD
    assert command(['git', 'branch', '--show-current']).strip() == BRANCH
    assert not command(['git', 'status', '--porcelain'])
    assert command(['git', 'diff', '--name-only', BASE, HEAD]).splitlines() == [
        'crates/sparq-bench/src/update_fuzz.rs'
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
