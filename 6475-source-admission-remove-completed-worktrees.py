import datetime
import json
import pathlib
import shutil
import subprocess

ROOT = pathlib.Path('/private/tmp/sparq-pr6049/.throughput-monitor')
OUT = ROOT / 'direct-6475/source-admission'
live = json.loads((OUT / 'completed-pr-revalidation-0013.json').read_text())
assert not live.get('errors')
records = []
for name, key, expected in [
    ('issue4246', 'p6469', 'ccded1b4898cf5b317a6591f6ff6108ced23123c'),
    ('issue6468', 'p6473', 'afa9c6489db0351855acd9297a6aa2c845434106'),
]:
    pr = live['data']['repository'][key]
    assert pr['state'] == 'MERGED' and pr['headRefOid'] == expected
    worktree = ROOT / 'worktrees' / name
    assert worktree.is_dir() and not worktree.is_symlink()

    def git(*args):
        return subprocess.run(['git', '-C', str(worktree), *args],
                              check=True, text=True, capture_output=True).stdout.strip()

    assert git('rev-parse', 'HEAD') == expected
    assert not git('status', '--porcelain=v1', '--untracked-files=all')
    branch = git('symbolic-ref', 'HEAD')
    common = git('rev-parse', '--git-common-dir')
    before = shutil.disk_usage(ROOT).free
    # Only known, ignored bytecode from the completed task is removed.
    for suffix in ['scripts/__pycache__', 'scripts/tests/__pycache__']:
        cache = worktree / suffix
        if cache.exists():
            assert cache.is_dir() and not cache.is_symlink()
            assert all(p.is_file() and not p.is_symlink() and p.suffix == '.pyc'
                       for p in cache.iterdir())
            shutil.rmtree(cache)
    assert not git('status', '--porcelain=v1', '--untracked-files=all', '--ignored=matching')
    subprocess.run(['git', '--git-dir=' + common, 'worktree', 'remove', str(worktree)],
                   check=True, capture_output=True, text=True)
    retained = subprocess.run(['git', '--git-dir=' + common, 'rev-parse', branch],
                              check=True, capture_output=True, text=True).stdout.strip()
    assert retained == expected and not worktree.exists()
    records.append({'path': str(worktree), 'branch': branch, 'head_retained': retained,
                    'merged_pr': key[1:], 'merge_commit': pr['mergeCommit']['oid'],
                    'free_before': before, 'free_after': shutil.disk_usage(ROOT).free})
    (OUT / 'completed-worktrees-removed.json').write_text(json.dumps({
        'at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'records': records, 'scope': 'Two clean completed task-owned worktrees only; '
        'no force, prune, branch deletion, shared checkout or production change.'
    }, indent=2) + '\n')
print(json.dumps(records, indent=2))
