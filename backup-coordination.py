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

OVERLAY_4246_REVISION = ROOT / 'direct-4246/revision'
for evidence_name in [*json.loads((OVERLAY_4246_REVISION / 'manifest.json').read_text())['files'], 'manifest.json', 'root-verification.json']:
    FILES['4246-revision-' + evidence_name.replace('/', '--')] = OVERLAY_4246_REVISION / evidence_name
FILES['4246-opus-review-cb638a42.json'] = OVERLAY_4246_REVISION / 'opus-review-cb638a42.json'

OVERLAY_4246_EXPERIMENTAL = ROOT / 'direct-4246/experimental'
for evidence_name in [*json.loads((OVERLAY_4246_EXPERIMENTAL / 'manifest.json').read_text())['files'], 'manifest.json', 'root-verification.json']:
    FILES['4246-experimental-' + evidence_name.replace('/', '--')] = OVERLAY_4246_EXPERIMENTAL / evidence_name
FILES['4246-opus-review-b86b5d5a.json'] = OVERLAY_4246_EXPERIMENTAL / 'opus-review-b86b5d5a.json'
VALIDATION_4246 = ROOT / 'direct-4246/validation-readiness'
for evidence_name in [*json.loads((VALIDATION_4246 / 'validation-readiness.json').read_text())['evidence_files'], 'validation-readiness.json']:
    FILES['4246-validation-readiness-' + evidence_name.replace('/', '--')] = VALIDATION_4246 / evidence_name
for evidence_name in ['publication-precheck.json', 'publication-postpush.json', 'pr-body.md', 'pr-created.txt', 'pr6469-initial.json', 'pr6469-runs-initial.json', 'pr6469-bench-initial-jobs.json']:
    FILES['4246-' + evidence_name] = ROOT / 'direct-4246' / evidence_name
VALIDATION_B86 = ROOT / 'direct-4246/validation-b86'
for evidence_name in [*json.loads((VALIDATION_B86 / 'manifest-final.json').read_text())['files'], 'manifest-final.json', 'root-verification.json']:
    FILES['4246-validation-b86-' + evidence_name.replace('/', '--')] = VALIDATION_B86 / evidence_name
for evidence_name in ['pr6469-ready-precheck.json', 'pr6469-wasm-feature-off-failed.log', 'pr6469-draft-gate-failed.log']:
    FILES['4246-' + evidence_name] = ROOT / 'direct-4246' / evidence_name
WASM_FAILURE_B86 = ROOT / 'direct-4246/wasm-failure-b86'
for evidence_name in [*json.loads((WASM_FAILURE_B86 / 'manifest.json').read_text())['files'], 'manifest.json', 'root-verification.json']:
    FILES['4246-wasm-failure-b86-' + evidence_name.replace('/', '--')] = WASM_FAILURE_B86 / evidence_name
FILES['4246-pr6469-checks-1405.json'] = ROOT / 'direct-4246/pr6469-checks-1405.json'
for evidence_name in ['source-retention/opus-review-6334b338.json', 'source-retention/opus-review-result.json', 'pr6469-checks-1438.json', 'pr6469-matrix-1438.json', 'pr6469-core-group-g02-b86.log', 'pr6469-deleted-feature-check-b86.json', 'pr6469-matrix-artifacts-b86.json', 'pr6469-matrix-selected-b86.zip', 'pr6469-matrix-results-1-b86.zip', 'pr6469-matrix-report-check-b86.json', 'pr6469-new-leg-b86-verification.json', '6334-prepush.json', '6334-postpush.json', 'pr-body-6334.md', '6334-body-verified.json', 'pr6469-runs-6334.json']:
    FILES['4246-' + evidence_name.replace('/', '--')] = ROOT / 'direct-4246' / evidence_name
for evidence_name in ['linux6334-proof-run.json', 'pr6469-checks-1458.json', 'linux6334-proof-job.log', 'linux6334-proof-step.txt', 'linux6334-declaration-generated.json', 'linux6334-declaration-reviewed.json', 'linux6334-proof-verification.json', 'declaration-commit-message.txt', 'declaration-head.txt', 'pr-body-declaration.md', 'declaration-prepush.json', 'declaration-postpush.json', 'declaration-ready-precheck.json', 'fa371-ready-verified.json', 'copilot-precheck-fa371.json', 'copilot-request.json', 'copilot-request-result-fa371.json', 'copilot-followup-fa371.json', 'fa371-full-runs.json', 'fa371-wasm-full-status.json']:
    FILES['4246-' + evidence_name.replace('/', '--')] = ROOT / 'direct-4246' / evidence_name
for evidence_name in ['review-fa371-1522-query.json', 'review-fa371-1522.json', 'checks-fa371-1522.json']:
    FILES['4246-' + evidence_name] = ROOT / 'direct-4246' / evidence_name
LAYOUT_REVIEW = ROOT / 'direct-4246/layout-review'
for evidence_name in [*json.loads((LAYOUT_REVIEW / 'manifest.json').read_text())['files'], 'manifest.json', 'root-verification.json', 'control-binary', 'opus-input.md']:
    FILES['4246-layout-review-' + evidence_name.replace('/', '--')] = LAYOUT_REVIEW / evidence_name
for evidence_name in ['opus-review-fa371.json', 'opus-review-result.json', 'opus-stderr.txt', 'cleanup-result.json']:
    FILES['4246-layout-review-' + evidence_name] = LAYOUT_REVIEW / evidence_name
FILES['4246-checks-fa371-1538.json'] = ROOT / 'direct-4246/checks-fa371-1538.json'
FILES['registry-pause-1538.json'] = ROOT / 'recovery-20260909/registry-pause-1538.json'
LAYOUT_COMMENT = ROOT / 'direct-4246/layout-comment'
for evidence_name in [*json.loads((LAYOUT_COMMENT / 'manifest.json').read_text())['files'], 'manifest.json', 'root-verification.json', 'prepush-remote.json', 'postpush-remote.json', 'pr-body.md', 'review-reply.json', 'resolve-thread.json', 'reply-precheck.json', 'review-reply-result.json', 'body-precheck.json', 'body-edit-result.txt', 'resolve-precheck.json', 'resolve-result.json', 'final-verified.json', 'initial-runs.json', 'initial-checks.json']:
    FILES['4246-layout-comment-' + evidence_name.replace('/', '--')] = LAYOUT_COMMENT / evidence_name
for evidence_name in ['checks-1558.json', 'review-1558.json']:
    FILES['4246-layout-comment-' + evidence_name] = LAYOUT_COMMENT / evidence_name
COUNTING_REVIEW = ROOT / 'direct-4246/counting-review'
for evidence_name in [*json.loads((COUNTING_REVIEW / 'manifest.json').read_text())['files'], 'manifest.json', 'freeze.json', 'root-verification.json']:
    FILES['4246-counting-review-' + evidence_name.replace('/', '--')] = COUNTING_REVIEW / evidence_name
COUNTING_FIX = ROOT / 'direct-4246/counting-fix'
for evidence_name in [*[item['path'] for item in json.loads((COUNTING_FIX / 'manifest.json').read_text())['files']], 'manifest.json', 'freeze.json']:
    FILES['4246-counting-fix-' + evidence_name.replace('/', '--')] = COUNTING_FIX / evidence_name
COUNTING_ADMISSION = ROOT / 'direct-4246/counting-admission'
for evidence_name in ['root-verification.json', 'head.diff', 'review-preface.md', 'opus-input.md']:
    FILES['4246-counting-admission-' + evidence_name] = COUNTING_ADMISSION / evidence_name
for evidence_name in ['pr-body-draft.md', 'review-comment-draft.md', 'publication-query.json', 'checks-1627.json', 'state-1627.json']:
    FILES['4246-counting-admission-' + evidence_name] = COUNTING_ADMISSION / evidence_name
for evidence_name in ['opus-review-ccded1b4.json', 'opus-review-result.json', 'opus-stderr.txt', 'prepush-1641.json', 'pr-body.md', 'review-comment.md', 'prepush-final.json', 'body-precheck.json', 'body-edit-result.txt', 'comment-precheck.json', 'comment-result.txt', 'publication-verified.json', 'new-head-checks.json', 'new-head-runs.json']:
    FILES['4246-counting-admission-' + evidence_name] = COUNTING_ADMISSION / evidence_name
FILES['registry-pause-1645.json'] = ROOT / 'recovery-20260909/registry-pause-1645.json'
SOURCE_RETENTION = ROOT / 'direct-4246/source-retention'
for evidence_name in [*json.loads((SOURCE_RETENTION / 'manifest.json').read_text())['files'], 'manifest.json', 'root-verification.json']:
    FILES['4246-source-retention-' + evidence_name.replace('/', '--')] = SOURCE_RETENTION / evidence_name
FILES['4246-pr6469-checks-1418.json'] = ROOT / 'direct-4246/pr6469-checks-1418.json'
for evidence_name in ['monitor-1418-query.json', 'monitor-1418.json', 'queued-1418.json', 'in-progress-1418.json', 'registry-pause-1418.json', 'nightly34327540479-jobs-1418.json']:
    FILES[evidence_name] = ROOT / 'recovery-20260909' / evidence_name

TRIAGE_6468 = ROOT / 'direct-6468'
for evidence_name in [*json.loads((TRIAGE_6468 / 'manifest.json').read_text())['files'], 'manifest.json', 'root-verification.json']:
    FILES['6468-diagnosis-' + evidence_name.replace('/', '--')] = TRIAGE_6468 / evidence_name
for evidence_name in ['6468-area-precheck.json', '6468-area-mutation.json', '6468-area-mutation-result.json']:
    FILES[evidence_name] = ROOT / 'recovery-20260909' / evidence_name

OPERATIONAL_TRIAGE = ROOT / 'recovery-20260909/operational-triage'
for evidence_name in [*json.loads((OPERATIONAL_TRIAGE / 'manifest.json').read_text())['files'], 'manifest.json', 'root-verification.json']:
    FILES['operational-triage-' + evidence_name.replace('/', '--')] = OPERATIONAL_TRIAGE / evidence_name
for evidence_name in ['operational-triage-root-query.json', 'operational-triage-root-read.json', 'registry-pause-revalidated.json', 'triage-missing-area-issue.md', 'triage-missing-area-created.txt', 'triage-missing-area-issue-verified.json']:
    FILES[evidence_name] = ROOT / 'recovery-20260909' / evidence_name

for evidence_name in ['checks-1740.json', 'review-1740.json', 'matrix-jobs-final.json', 'matrix-artifacts-final.json', 'gate-final.json', 'effective-main-rules.json', 'matrix-g02-final.log', 'matrix-selected-final.zip', 'matrix-results-1-final.zip', 'matrix-report-final.json', 'new-feature-check-final.json', 'matrix-feature-execution.log', 'docs-jobs-final.json', 'full-validation-verification.json']:
    FILES["4246-counting-admission-" + evidence_name] = ROOT / "direct-4246/counting-admission" / evidence_name
for evidence_name in ["opus-input.md", "input-manifest.json", "head-without-generated-lock.diff", "performance-checks.json"]:
    FILES["4246-perf-admission-" + evidence_name] = ROOT / "direct-4246/perf-admission" / evidence_name

for evidence_name in ['benchmark-jobs.json', 'body-edit-result.txt', 'body-precheck.json', 'comment-precheck.json', 'comment-result.txt', 'enqueue-checks.json', 'enqueue-input.json', 'enqueue-precheck.json', 'enqueue-result.json', 'enqueue-root-verification.json', 'enqueue-rules.json', 'final-review-comment.md', 'in-progress-runs.json', 'opus-review-ccded-final-perf.json', 'opus-review-result.json', 'opus-stderr.txt', 'pr-body-final-draft.md', 'pr-body-final.md', 'queue-query.json', 'queue-verified.json', 'queued-runs.json', 'registry-pause.json', 'review-current.json']:
    FILES["4246-perf-admission-" + evidence_name] = ROOT / "direct-4246/perf-admission" / evidence_name

FILES["4246-perf-admission-queue-initial-runs.json"] = ROOT / "direct-4246/perf-admission/queue-initial-runs.json"
FILES["6468-current-1802.json"] = ROOT / "direct-6468/current-1802.json"

for evidence_name in ["queue-1818.json", "queue-runs-1818.json", "queue-matrix-jobs-1818.json", "queue-ci-jobs-1818.json"]:
    FILES["4246-perf-admission-" + evidence_name] = ROOT / "direct-4246/perf-admission" / evidence_name
for evidence_name in ["report.json", "manifest.json"]:
    FILES["6468-next-design-" + evidence_name] = ROOT / "direct-6468/next-design" / evidence_name
for evidence_name in ["label-before-review.json", "issue5016-before-review.json", "source-verification.json", "create-input.json", "opus-input.md", "input-manifest.json"]:
    FILES["6468-label-maintenance-" + evidence_name] = ROOT / "direct-6468/label-maintenance" / evidence_name

for evidence_name in ['queue-1836.json', 'queue-runs-1836.json', 'issue4246-before-close.json', 'main-source-proof.json', 'issue4246-close-comment.md', 'issue4246-comment-precheck.json', 'issue4246-comment-result.txt', 'issue4246-close-precheck.json', 'issue4246-close-result.txt', 'issue4246-closed-verified.json', 'main-runs-1859.json']:
    FILES["4246-perf-admission-" + evidence_name] = ROOT / "direct-4246/perf-admission" / evidence_name

for evidence_name in ['comment-result.txt', 'comment-verified.json', 'create-input.json', 'create-readback.json', 'create-result.json', 'docs-drift-comment-result.txt', 'docs-drift-comment-verified.json', 'docs-drift-comment.md', 'docs-drift-dedupe-query-v2.json', 'docs-drift-dedupe-query.json', 'docs-drift-dedupe-v2.json', 'docs-drift-dedupe.json', 'docs-drift-issue5412-precheck.json', 'input-manifest.json', 'issue-comment-precheck.json', 'issue5016-before-review.json', 'label-before-review.json', 'maintenance-comment.md', 'maintenance-verified.json', 'open-pr-paths-page1.json', 'open-pr-paths-query-v2-page2.json', 'open-pr-paths-query-v2.json', 'open-pr-paths-query.json', 'open-pr-paths-root-verification.json', 'open-pr-paths-v2-page1.json', 'open-pr-paths-v2-page2.json', 'opus-input.md', 'opus-review-result.json', 'opus-review.json', 'opus-stderr.txt', 'pr5907-files-page2.json', 'pr5907-files-page3.json', 'pr5907-files-page4.json', 'pr5907-files-page5.json', 'pr5907-files-page6.json', 'pr5907-files-page7.json', 'pr5907-files-page8.json', 'pr5907-files-query2.json', 'pr5907-files-query3.json', 'pr5907-files-query4.json', 'pr5907-files-query5.json', 'pr5907-files-query6.json', 'pr5907-files-query7.json', 'pr5907-files-query8.json', 'precreate-exact-label.json', 'precreate-heads-query1.json', 'precreate-heads-query2.json', 'precreate-heads1.json', 'precreate-heads2.json', 'precreate-registry-pause.json', 'precreate-root-verification.json', 'scheduled-runs-after-create.json', 'source-revalidated-cea4414.json', 'source-verification.json']:
    FILES["6468-label-maintenance-" + evidence_name] = ROOT / "direct-6468/label-maintenance" / evidence_name

for evidence_name in ['5457-base-to-main-test_triage_area.py.diff', '5457-base-to-main-triage-area.py.diff', '6095-base-to-main-test_triage_area.py.diff', '6095-base-to-main-triage-area.py.diff', 'base-composition.json', 'budget.json', 'p5457.diff', 'p6095.diff', 'pr5457-checks.json', 'pr6095-checks.json', 'report.json', 'report.md', 'reviews-and-holds.json', 'source/main/scripts/tests/test_triage_area.py', 'source/main/scripts/triage-area.py', 'source/pr5457/scripts/tests/test_triage_area.py', 'source/pr5457/scripts/triage-area.py', 'source/pr6095/scripts/tests/test_triage_area.py', 'source/pr6095/scripts/triage-area.py', 'manifest.json', 'root-verification.json']:
    FILES["6468-conflict-assessment-" + evidence_name.replace("/", "--")] = ROOT / "direct-6468/conflict-assessment" / evidence_name

for evidence_name in ["pr5457-conflict.json", "pr6095-conflict.json"]:
    FILES["6468-" + evidence_name] = ROOT / "direct-6468" / evidence_name

FILES["6468-source-admission-review-preface-draft.md"] = ROOT / "direct-6468/source-admission/review-preface-draft.md"

for evidence_name in ['bd-self-test-exact-head.log', 'bd-self-test.log', 'changed-files.txt', 'ci-wiring.txt', 'commit.txt', 'composition-check.log', 'controls/cross-associate-labels/mutation.diff', 'controls/cross-associate-labels/test.log', 'controls/cross-associate-labels/triage-area.py', 'controls/delete-rule/mutation.diff', 'controls/delete-rule/test.log', 'controls/delete-rule/triage-area.py', 'controls/disable-global-guard/mutation.diff', 'controls/disable-global-guard/test.log', 'controls/disable-global-guard/triage-area.py', 'controls/drop-tier-evidence/mutation.diff', 'controls/drop-tier-evidence/test.log', 'controls/drop-tier-evidence/triage-area.py', 'controls/drop-title-anchor/mutation.diff', 'controls/drop-title-anchor/test.log', 'controls/drop-title-anchor/triage-area.py', 'controls/guard-only-writable-prefix/mutation.diff', 'controls/guard-only-writable-prefix/test.log', 'controls/guard-only-writable-prefix/triage-area.py', 'controls/unescape-record-newlines/mutation.diff', 'controls/unescape-record-newlines/test.log', 'controls/unescape-record-newlines/triage-area.py', 'controls/wrong-row-number/mutation.diff', 'controls/wrong-row-number/test.log', 'controls/wrong-row-number/triage-area.py', 'controls-run.log', 'controls.json', 'critical-source.py.txt', 'fallback-source.py.txt', 'focused-tests.py.txt', 'freeze.py', 'full.diff', 'preflight-result.json', 'preflight.log', 'report.json', 'report.md', 'review-packet.md', 'run-controls.py', 'scope-composition.json', 'self-test-commands.json', 'source/.github/workflows/docs-quality.yml', 'source/scripts/bd-to-issues.py', 'source/scripts/preflight.py', 'source/scripts/tests/test_triage_area.py', 'source/scripts/triage-area.py', 'test-counts.json', 'triage-area-tests.log', 'triage-self-test-exact-head.log', 'triage-self-test.log', 'manifest.json', 'freeze.json']:
    FILES["6468-implementation-" + evidence_name.replace("/", "--")] = ROOT / "direct-6468/implementation" / evidence_name

for evidence_name in ['root-verification.json', 'opus-input.md', 'input-manifest.json', 'repository-visibility.json', 'payload-risk-audit.json']:
    FILES["6468-source-admission-" + evidence_name] = ROOT / "direct-6468/source-admission" / evidence_name

for evidence_name in ["opus-review-5f758d2a.json", "opus-review-result.json", "opus-stderr.txt"]:
    FILES["6468-source-admission-" + evidence_name] = ROOT / "direct-6468/source-admission" / evidence_name
FILES["6468-label-maintenance-scheduled-runs-1920.json"] = ROOT / "direct-6468/label-maintenance/scheduled-runs-1920.json"
FILES["4246-perf-admission-main-runs-1920.json"] = ROOT / "direct-4246/perf-admission/main-runs-1920.json"

for evidence_name in ["main-ci-jobs-1924.json", "main-matrix-jobs-1924.json"]:
    FILES["4246-perf-admission-" + evidence_name] = ROOT / "direct-4246/perf-admission" / evidence_name

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
