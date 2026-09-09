from pathlib import Path
import ast
import difflib
import hashlib
import json
import subprocess

ROOT = Path(__file__).resolve().parent
WT = Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue6468')
ORIGINAL = ROOT.parent / 'implementation'
BASE = '5f758d2afed6524cf191820463934c2213ddcfda'
MAIN = 'cea4414b39225b1240f36d968ba1549e700cd32f'

def sha(data):
    return hashlib.sha256(data).hexdigest()

def git(*args):
    return subprocess.check_output(['git', *args], cwd=WT)

head = git('rev-parse', 'HEAD').decode().strip()
assert git('status', '--porcelain') == b''
assert git('diff', BASE, head) == (ROOT / 'delta.diff').read_bytes()
assert git('diff', MAIN, head) == (ROOT / 'full.diff').read_bytes()
for entry in json.loads((ORIGINAL / 'manifest.json').read_text())['files']:
    assert sha((ORIGINAL / entry['path']).read_bytes()) == entry['sha256'], entry['path']
controls = json.loads((ROOT / 'controls.json').read_text())
assert not controls['survivors']
assert controls['candidate_sha256'] == sha((WT / 'scripts/triage-area.py').read_bytes())
assert all(c['errors'] == 0 for c in controls['controls'])
peer = ROOT.parent / 'conflict-assessment/p6095.diff'
command = ['git', 'apply', '--check', str(peer)]
check = subprocess.run(command, cwd=WT, capture_output=True, text=True)
(ROOT / 'composition-check.log').write_text(check.stdout + check.stderr)
assert check.returncode == 0
whitespace = subprocess.run(['git', 'diff', '--check', MAIN, head], cwd=WT, capture_output=True, text=True)
(ROOT / 'diff-check.log').write_text(whitespace.stdout + whitespace.stderr)
assert whitespace.returncode == 0
old = git('show', BASE + ':scripts/triage-area.py').decode()
new = (WT / 'scripts/triage-area.py').read_text()
for token, end in [('_DECL =', '\ndef classify('), ('RULES = [', '\n\n# --- T0:')]:
    assert old[old.index(token):old.index(end)] == new[new.index(token):new.index(end)]
test_old = git('show', BASE + ':scripts/tests/test_triage_area.py').decode()
test_new = (WT / 'scripts/tests/test_triage_area.py').read_text()
for name in ['test_every_rule_has_a_witness_that_actually_matches',
             'test_a_title_scoped_rule_never_fires_from_the_body_alone',
             'test_a_text_scoped_rule_does_fire_from_the_body',
             'test_the_scope_immune_rules_are_exactly_the_fully_anchored_ones']:
    def body(source):
        return next(ast.get_source_segment(source, node) for node in ast.walk(ast.parse(source))
                    if isinstance(node, ast.FunctionDef) and node.name == name)
    assert body(test_old) == body(test_new)
for relative in ['scripts/triage-area.py', 'scripts/tests/test_triage_area.py',
                 '.github/workflows/docs-quality.yml', '.github/workflows/triage-area.yml']:
    destination = ROOT / 'source' / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes((WT / relative).read_bytes())
(ROOT / 'commit.txt').write_bytes(git('show', '--no-patch', '--format=fuller', head))
review = ROOT.parent / 'source-admission/opus-review-result.json'
(ROOT / 'prior-opus-review.json').write_bytes(review.read_bytes())
runner_delta = ''.join(difflib.unified_diff(
    (ORIGINAL / 'run-controls.py').read_text().splitlines(True),
    (ROOT / 'run-controls.py').read_text().splitlines(True),
    fromfile='original/run-controls.py', tofile='followup/run-controls.py'))
(ROOT / 'control-runner.delta').write_text(runner_delta)
ci = (WT / '.github/workflows/docs-quality.yml').read_text().splitlines()
scheduled = (WT / '.github/workflows/triage-area.yml').read_text().splitlines()
# Full on block, job setup/version, and exact bare commands; comments retain their
# original line numbers. Complete YAML also lives in the immutable source snapshot.
wiring = 'docs-quality.yml (exact source lines):\n' + '\n'.join(
    f'{i}: {line}' for i, line in enumerate(ci, 1)
    if 142 <= i <= 158 or 175 <= i <= 188 or 625 <= i <= 628)
wiring += '\n\ntriage-area.yml (exact source lines):\n' + '\n'.join(
    f'{i}: {line}' for i, line in enumerate(scheduled, 1) if 74 <= i <= 122)
(ROOT / 'ci-wiring.txt').write_text(wiring + '\n')
counts = {'suite_tests': 45, 'new_class_tests': 8, 'classifier_assertions': 17,
          'bd_assertions': 114, 'controls': len(controls['controls']),
          'control_test_executions': sum(c['tests'] for c in controls['controls']),
          'survivors': 0, 'compile_failures': 0, 'test_errors': 0}
report = {
    'head': head, 'base': BASE, 'main': MAIN, 'clean': True,
    'actual_model': 'OpenAI GPT-6 Astra xhigh; no new independent model call.',
    'delta': '+27/-9 in the same two Python files; runtime change is operator wording only.',
    'addressed': {
        'NB1_NB3': 'Fetched area-label set count and possible incomplete enumeration are explicit. Do not create from this failure; exact repository-label GET first, then separately reviewed maintenance for verified existing-crate provisioning. No creation/write behavior changed.',
        'NB2': 'Real benchmark and website collision titles both assert ci and T1 triage-area. Moving the rule to the end fails the collision test.',
        'NB5': 'Drop-title-anchor executes all45 tests, failing scope equality and the title negative. Separate test-metadata control removes only triage-area from ANCHORED_ONLY and fails exactly the unchanged equality property.',
        'NB6': 'No-forged-workflow-line and one UNKNOWN_AREA-prefix assertions precede decoding. Exact newline-unescape mutant now fails assertFalse(any line starts ::), failures1/errors0; prefix count alone would not detect its extra unprefixed lines.',
        'NB7': 'Actual unknown-label CLI case includes apply=False. Default helper path remains --apply and preserves previous fixtures.',
        'NB4': 'Record cap deliberately declined: retain the complete finite offending-record set under the existing fetched-plan bounds; no new truncation policy.',
        'NB8': 'Fixtures use real CRATES discovery and couple to sparq-wrapper-gen/sparq-wrapper existence. A crate rename requires updating these fixtures with the actual ownership contract, not blindly treating failures as routing regressions.',
        'NB9': 'Local Python3.14.5 captured. docs-quality quick-gates explicitly setup-python3.12, sufficient for removeprefix; actual Linux run still required. Scheduled lane uses its ubuntu-latest Python, not a locally measured CI interpreter.',
    },
    'validation': counts,
    'preflight': 'Only failure remains Bash3 mapfile in privacy checker. Other applicable checks pass; no suppressions/installs. Full and delta committed diffs are byte-identical to captured inputs.',
    'consumer_audit': 'git grep over tracked HEAD5f758 and origin/maincea441 for both old error strings found no consumers. HEAD had no matches; main had exactly the two old producer print lines. No log/transcript scan or remote read.',
    'ci_audit': 'Complete docs-quality on block has no paths/paths-ignore; pull_request types include opened/synchronize/reopened/ready_for_review, plus merge_group and push/main. quick-gates invokes both classifier suites. Scheduled triage-area runs both suites before --apply with set -euo pipefail.',
    'composition': 'Read-only6095 full patch check passes again. T0 region, RULES and legacy property-loop bodies unchanged from5f758. No old PR imports, census/workflow/budget changes or remote actions.',
    'original_evidence': {'packet_sha256': sha((ORIGINAL / 'review-packet.md').read_bytes()),
                          'manifest_verified_files': 54, 'unchanged': True,
                          'review_sha256': sha(review.read_bytes())},
    'limits': 'No live board replay, label creation, registry action, builds, network installs or model calls. Known500-label fetch limitation remains separate. Linux privacy gate and focused independent delta review remain outstanding.',
}
(ROOT / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
summary = (f'Head `{head}`; delta from `{BASE}` is two files +27/-9. Actual Astra xhigh.\n\n'
    'NB1/3: wording now names the fetched set/count and incomplete-fetch possibility, explicitly prohibits creating a label from this failure, and directs an exact GET lookup before separate reviewed maintenance. No guard/write policy changes.\n\n'
    'NB2/5/6/7: both real title collisions pass; late-rule control fails. Drop-anchor and the separate ANCHORED_ONLY metadata control run the full45-test suite. The newline mutant now fails the workflow-line assertion before JSON decoding. Dry-run is exercised through actual main. All10 controls killed,154 test executions, zero errors/compile failures. Full45 tests and17/114 self-test assertions pass.\n\n'
    'NB4: retain all finite offender records; no cap. NB8: real-crate fixtures intentionally couple to wrapper/gen membership. NB9: local Python3.14.5; CI explicitly pins3.12. Author preflight still fails only at Bash3 privacy/mapfile; Linux must execute that gate.\n\n'
    'Original54 evidence files verified unchanged. Frozen6095 patch still applies through read-only --check. No source was imported from6095 or5457. No remote action/model call/build.\n\n'
    'Review context: use the frozen original implementation packet (SHA256 '
    + report['original_evidence']['packet_sha256'] + ') and its approve_for_validation result; this packet is the follow-up delta. '
    'Unchanged full classifier/rule/parser context and legacy test loops are omitted here. Complete current sources, full diff, all control sources/logs and all45-test output are in this manifest.\n')
(ROOT / 'report.md').write_text(summary)
key_controls = ('move-rule-late', 'drop-title-anchor', 'remove-anchor-metadata', 'unescape-record-newlines')
control_evidence = ''
for name in key_controls:
    record = next(c for c in controls['controls'] if c['name'] == name)
    control_evidence += json.dumps({k: record[k] for k in ['name','metadata_control','tests','failures','errors']}) + '\n'
    log = (ROOT / 'controls' / name / 'test.log').read_text()
    failure = log[log.index('======================================================================'):]
    control_evidence += failure + '\n'
(ROOT / 'key-control-evidence.txt').write_text(control_evidence)
parts = [
    ('Focused report and prior context', 'report.md', 'text'),
    ('Exact follow-up diff', 'delta.diff', 'diff'),
    ('Changed executable control runner', 'control-runner.delta', 'diff'),
    ('Actual targeted control failures', 'key-control-evidence.txt', 'text'),
    ('Complete trigger and execution-order evidence', 'ci-wiring.txt', 'text'),
    ('Full exact-source migration self-test output', 'bd-self-test.log', 'text'),
    ('Executed self-test/preflight commands', 'commands.json', 'json'),
    ('Local interpreter', 'python-version.txt', 'text'),
]
packet = '# #6468 focused follow-up review\n\n'
for title, filename, language in parts:
    packet += f'## {title}\n\n`{filename}`\n\n```{language}\n' + (ROOT / filename).read_text() + '\n```\n\n'
assert len(packet.encode()) <= 32768, len(packet.encode())
(ROOT / 'review-packet.md').write_text(packet)
entries = []
for path in sorted(ROOT.rglob('*')):
    if path.is_file() and path.name not in ('manifest.json','freeze.json'):
        entries.append({'path': str(path.relative_to(ROOT)), 'bytes': path.stat().st_size,
                        'sha256': sha(path.read_bytes())})
(ROOT / 'manifest.json').write_text(json.dumps({'head': head, 'files': entries}, indent=2) + '\n')
freeze = {'head': head, 'base': BASE, 'clean': True, 'packet_path': str(ROOT / 'review-packet.md'),
          'packet_bytes': len(packet.encode()), 'packet_sha256': sha(packet.encode()),
          'manifest_files': len(entries), 'manifest_sha256': sha((ROOT / 'manifest.json').read_bytes())}
(ROOT / 'freeze.json').write_text(json.dumps(freeze, indent=2) + '\n')
print(json.dumps(freeze, indent=2))
