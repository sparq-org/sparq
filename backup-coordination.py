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
    '6436-review-packet-2f3d1773.md': ROOT / 'direct-6436/review-2f3d1773bb3cebd36186692025912a8377c85db6.md',
    '6436-opus-review-2f3d1773.json': ROOT / 'direct-6436/opus-review-2f3d1773.json',
    '6436-manifest-2f3d1773.json': ROOT / 'direct-6436/manifest-2f3d1773.json',
    '6462-control-results.json': ROOT / 'recovery-20260908/6462-control-results.json',
    '6462-tests.txt': ROOT / 'recovery-20260908/6462-tests.txt',
    '6462-old-query-control.py': ROOT / 'recovery-20260908/6462-old-query-control.py',
    '6462-review-ed9689d5.md': ROOT / 'recovery-20260908/6462-review-ed9689d5.md',
    '6462-review-manifest.json': ROOT / 'recovery-20260908/6462-review-manifest.json',
    '6436-nightly-real-jobs.json': ROOT / 'recovery-20260908/nightly-34099890455-jobs-page1.json',
    '6436-mutants-parse-10010502882.zip': ROOT / 'recovery-20260908/mutants-parse-10010502882.zip',
    '6436-mutants-engine-10036242958.zip': ROOT / 'recovery-20260908/mutants-engine-10036242958.zip',
    '6462-opus-review-ed9689d5.json': ROOT / 'recovery-20260908/6462-opus-review-ed9689d5.json',
    '6462-rereview-5640d06b.md': ROOT / 'recovery-20260908/6462-rereview-5640d06b.md',
    '6462-rereview-manifest.json': ROOT / 'recovery-20260908/6462-rereview-manifest.json',
    '6462-bot-identity-proof.json': ROOT / 'recovery-20260908/6462-bot-identity-proof.json',
    '6462-final-control-results.json': ROOT / 'recovery-20260908/6462-final-control-results.json',
    '6462-tests-final.txt': ROOT / 'recovery-20260908/6462-tests-final.txt',
    '6448-main-source-proof.json': ROOT / 'recovery-20260908/6448-main-source-proof.json',
    '6448-queue-gate-final.json': ROOT / 'recovery-20260908/6448-queue-gate-final.json',
    '6462-opus-rereview-5640d06b.json': ROOT / 'recovery-20260908/6462-opus-rereview-5640d06b.json',
    '6463-pr-body.md': ROOT / 'recovery-20260908/6462-pr-body.md',
    '6463-initial-state.json': ROOT / 'recovery-20260908/6463-initial-state.json',
    '6436-followup-frozen-packet.md': ROOT / 'direct-6436/followup/review-complete-0fc2faf6d4255e9e27b8a17bc43e8ba848c29368.md',
    '6436-followup-input.md': ROOT / 'direct-6436/followup/review-input-0fc2faf6.md',
    '6436-followup-manifest.json': ROOT / 'direct-6436/followup/manifest-complete.json',
    '6436-followup-input-manifest.json': ROOT / 'direct-6436/followup/review-input-manifest.json',
    '6436-real-parse-job.log': ROOT / 'recovery-20260908/nightly-parse-job.log',
    '6436-opus-review-0fc2faf6.json': ROOT / 'direct-6436/followup/opus-review-0fc2faf6.json',
    '6464-pr-body.md': ROOT / 'direct-6436/followup/pr-body.md',
    '6464-main-source-proof.json': ROOT / 'direct-6436/followup/main-source-proof.json',
    '6464-queue-docs-quality.log': ROOT / 'direct-6436/followup/pr6464-queue-docs-quality.log',
    '6464-landed-runs.json': ROOT / 'direct-6436/followup/pr6464-queue-runs.json',
    '6464-resolution-comment.md': ROOT / 'direct-6436/followup/issue6436-close-comment.md',
    '5983-triage-metadata.json': ROOT / 'recovery-20260908/pr5983-triage.json',
    '5983-feasibility.json': ROOT / 'direct-5983/feasibility.json',
    '6465-issue.md': ROOT / 'direct-5983/topk-recovery-issue.md',
    '5983-findings-comment.md': ROOT / 'direct-5983/pr5983-findings-comment.md',
    '5983-stage1-report.json': ROOT / 'direct-5983/implementation/report.json',
    '5983-stage1-manifest.json': ROOT / 'direct-5983/implementation/manifest.json',
    '5983-stage1-recovery.diff': ROOT / 'direct-5983/implementation/recovery.diff',
    '5983-stage1-astra-repair.diff': ROOT / 'direct-5983/implementation/astra-repair.diff',
    '5983-stage1-t1-raw.txt': ROOT / 'direct-5983/implementation/t1-raw.txt',
    '5983-stage1-t1-main-fallback.txt': ROOT / 'direct-5983/implementation/t1-main-fallback.txt',
    '5983-stage1-resource-raw.txt': ROOT / 'direct-5983/implementation/resource-raw.txt',
    '5983-stage1-focused-repaired.txt': ROOT / 'direct-5983/implementation/focused-repaired.txt',
    '5983-stage1-existing-orderby-tests.txt': ROOT / 'direct-5983/implementation/existing-orderby-tests.txt',
    '5983-stage1-mutation-report.json': ROOT / 'direct-5983/implementation/mutation-report.json',
    '5983-stage1-mutate.py': ROOT / 'direct-5983/implementation/mutate.py',
    '5983-summary.md': ROOT / 'direct-5983/summary.md',
    '5983-manifest.json': ROOT / 'direct-5983/manifest.json',
    '5983-comparison.json': ROOT / 'direct-5983/comparison.json',
    '5983-original.diff': ROOT / 'direct-5983/pr.diff',
    'nightly34202612161-freshness.log': ROOT / 'direct-6436/followup/nightly34202612161-freshness.log',
    'nightly34202612161-jobs.json': ROOT / 'direct-6436/followup/nightly34202612161-jobs.json',
    'performance-frontier.json': ROOT / 'recovery-20260908/perf-title-frontier.json',
    '6463-main-source-proof.json': ROOT / 'recovery-20260908/6463-main-source-proof.json',
    '6463-landed-runs.json': ROOT / 'recovery-20260908/6463-landed-runs.json',
    '6463-resolution-comment.md': ROOT / 'recovery-20260908/6462-close-comment.md',
    '6464-enqueue-result.json': ROOT / 'direct-6436/followup/pr6464-enqueue-result.json',
    'batch-merge-34193009779-failed.log': ROOT / 'recovery-20260908/batch-merge-34193009779-failed.log',
    'release-plz-34193009813-failed.log': ROOT / 'recovery-20260908/release-plz-34193009813-failed.log',
    '6464-pr-docs-quality.log': ROOT / 'direct-6436/followup/pr6464-docs-quality.log',
    '6463-pr-docs-quality.log': ROOT / 'recovery-20260908/6463-pr-docs-quality.log',
    '6463-queue-docs-quality.log': ROOT / 'recovery-20260908/6463-queue-docs-quality.log',
    '6463-enqueue-result.json': ROOT / 'recovery-20260908/6463-enqueue-result.json',
    '6464-current-runs.json': ROOT / 'direct-6436/followup/pr6464-runs.json',
}

# Preserve the frozen semantic evidence; exclude build caches and live review output.
SEMANTIC = ROOT / 'direct-5983/semantic'
for evidence_name in [*json.loads((SEMANTIC / 'manifest.json').read_text())['files'], 'manifest.json', 'root-verification.json']:
    FILES['5983-semantic-' + evidence_name] = SEMANTIC / evidence_name

FILES['5983-opus-review-9e8bdfc9.json'] = ROOT / 'direct-5983/semantic/opus-review-9e8bdfc9.json'

# Frozen bounded diagnostic evidence only; build target/cache excluded by manifest.
DIAGNOSTIC = ROOT / 'direct-5983/diagnostic'
for evidence_name in [*json.loads((DIAGNOSTIC / 'manifest.json').read_text())['files'], 'manifest.json', 'root-verification.json']:
    FILES['5983-diagnostic-' + evidence_name.replace('/', '--')] = DIAGNOSTIC / evidence_name
FILES['5983-diagnostic-findings-comment.md'] = ROOT / 'direct-5983/diagnostic-findings-comment.md'

EARLY_TIE = ROOT / 'direct-5983/early-tie'
for evidence_name in [*json.loads((EARLY_TIE / 'manifest.json').read_text())['files'], 'manifest.json', 'root-verification.json']:
    FILES['5983-early-tie-' + evidence_name.replace('/', '--')] = EARLY_TIE / evidence_name

for evidence_prefix, evidence_subdir in [('5983-drained-design-', 'direct-5983/drained-design'), ('sweeper34213698157-', 'recovery-20260908/sweeper-34213698157')]:
    evidence_root = ROOT / evidence_subdir
    for evidence_name in [*json.loads((evidence_root / 'manifest.json').read_text())['files'], 'manifest.json']:
        FILES[evidence_prefix + evidence_name.replace('/', '--')] = evidence_root / evidence_name

LAZY_PROBES = ROOT / 'direct-5983/lazy-probes'
for evidence_name in [*json.loads((LAZY_PROBES / 'manifest.json').read_text())['files'], 'manifest.json', 'root-verification.json']:
    FILES['5983-lazy-probes-' + evidence_name.replace('/', '--')] = LAZY_PROBES / evidence_name
for evidence_name in ['overlay-cost-dedupe-query.json', 'overlay-cost-dedupe.json', 'overlay4246-read-query.json', 'overlay4246-read.json', 'overlay4246-start-comment.md', 'main-a42a9e89-runs.json']:
    FILES[evidence_name] = ROOT / evidence_name
FILES['5983-final-no-go-comment.md'] = ROOT / 'direct-5983/final-no-go-comment.md'

OVERLAY_4246 = ROOT / 'direct-4246'
for evidence_name in [*json.loads((OVERLAY_4246 / 'manifest.json').read_text())['files'], 'manifest.json', 'root-verification.json']:
    FILES['4246-' + evidence_name.replace('/', '--')] = OVERLAY_4246 / evidence_name

FILES['4246-opus-review-acfa31cf.json'] = ROOT / 'direct-4246/opus-review-acfa31cf.json'

def git(*args, data=None):
    return subprocess.run(['git', '--git-dir=' + GIT_DIR, *args], input=data,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True).stdout

def main():
    json.loads(FILES['direct-coordination.json'].read_text())
    refs = git('for-each-ref', '--format=%(refname) %(objectname)', REF).decode().splitlines()
    old = next((line.split()[1] for line in refs if line.split()[0] == REF), None)
    object_format = git('rev-parse', '--show-object-format').decode().strip()
    old_entries = {}
    if old:
        for record in git('ls-tree', '-z', old).split(b'\0'):
            if record:
                metadata, name = record.split(b'\t', 1)
                old_entries[name.decode()] = metadata.decode().split()[2]
    entries = []
    digests = {}
    for name, path in sorted(FILES.items()):
        content = path.read_bytes()
        blob = hashlib.new(object_format)
        blob.update(f'blob {len(content)}\0'.encode())
        blob.update(content)
        oid = blob.hexdigest()
        # An identical blob in the prior committed tree already exists locally.
        if old_entries.get(name) != oid:
            written = git('hash-object', '-w', '--stdin', data=content).decode().strip()
            assert written == oid, name
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
