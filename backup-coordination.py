#!/usr/bin/env python3
"""Save selected local coordination state in a private Git ref, without a checkout."""
import hashlib
import json
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parent
GIT_DIR = '/Users/jesght/Documents/ChatGPT/sparq/upstream/.git'
REF = 'refs/heads/codex/coordinator-local-state'
FILES = {
    'direct-coordination.json': ROOT / 'direct-coordination.json',
    'backup-coordination.py': pathlib.Path(__file__).resolve(),
    'current-work-monitor-query.json': ROOT / 'current-work-monitor-query.json',
    'current-work-monitor-snapshot.json': ROOT / 'current-work-monitor-snapshot.json',
    'current-main-rules.json': ROOT / 'current-main-rules.json',
    'registry-pause-snapshot.json': ROOT / 'registry-pause-snapshot.json',
    'recovery-source-proof.json': ROOT / 'recovery-20260908/source-proof.json',
    'recovery-rearm-scheduled-runs.json': ROOT / 'recovery-20260908/rearm-scheduled-runs.json',
    '6448-queue-docs-quality.log': ROOT / 'recovery-20260908/6448-queue-docs-quality.log',
    '6462-schema-and-dedupe.json': ROOT / 'recovery-20260908/sweeper-schema-and-dedupe.json',
    '6462-issue.md': ROOT / 'recovery-20260908/actor-id-issue.md',
    'sweeper-34168680735-failed.log': ROOT / 'recovery-20260908/rearm-34168680735-failed.log',
    'sweeper-34175520707.log': ROOT / 'recovery-20260908/rearm-34175520707.log',
}

def git(*args, data=None):
    return subprocess.run(['git', '--git-dir=' + GIT_DIR, *args], input=data,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True).stdout

def main():
    json.loads(FILES['direct-coordination.json'].read_text())
    refs = git('for-each-ref', '--format=%(refname) %(objectname)', REF).decode().splitlines()
    old = next((line.split()[1] for line in refs if line.split()[0] == REF), None)
    entries = []
    digests = {}
    for name, path in sorted(FILES.items()):
        content = path.read_bytes()
        oid = git('hash-object', '-w', '--stdin', data=content).decode().strip()
        entries.append(f'100644 blob {oid}\t{name}\n')
        digests[name] = hashlib.sha256(content).hexdigest()
    tree = git('mktree', data=''.join(entries).encode()).decode().strip()
    if old and git('rev-parse', old + '^{tree}').decode().strip() == tree:
        print(json.dumps({'commit': old, 'unchanged': True}))
        return
    parent = ['-p', old] if old else []
    commit = git('-c', 'commit.gpgsign=false', 'commit-tree', tree, *parent,
                 '-m', 'Save local direct-coordination checkpoint').decode().strip()
    git('update-ref', REF, commit, old or '0' * 40)
    assert git('show', f'{REF}:direct-coordination.json') == FILES['direct-coordination.json'].read_bytes()
    print(json.dumps({'ref': REF, 'commit': commit, 'sha256': digests}, indent=2))

if __name__ == '__main__':
    main()
