from pathlib import Path
import ast
import hashlib
import json
import subprocess
import time

ROOT = Path(__file__).resolve().parent
WT = Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue6468')
BASE = 'cea4414b39225b1240f36d968ba1549e700cd32f'

def sha(data):
    return hashlib.sha256(data).hexdigest()

def git(*args):
    return subprocess.check_output(['git', *args], cwd=WT)

head = git('rev-parse', 'HEAD').decode().strip()
assert git('status', '--porcelain') == b''
assert git('diff', BASE, head) == (ROOT / 'full.diff').read_bytes()
for relative in ('scripts/triage-area.py', 'scripts/tests/test_triage_area.py'):
    assert git('show', head + ':' + relative) == (ROOT / 'source' / relative).read_bytes()
controls = json.loads((ROOT / 'controls.json').read_text())
assert controls['candidate_sha256'] == sha((WT / 'scripts/triage-area.py').read_bytes())
assert not controls['survivors']
test_times = {
    'classifier_tests_tool_wall_seconds': 0.555900042,
    'first_self_tests_and_controls_tool_wall_seconds': 0.996580417,
    'controls_in_process_seconds': controls['elapsed_seconds'],
    'preflight_seconds': json.loads((ROOT / 'preflight-result.json').read_text())['seconds'],
    'exact_head_self_tests': json.loads((ROOT / 'self-test-commands.json').read_text()),
    'validation_window_seconds_from_first_test_log': time.time() - (ROOT / 'triage-area-tests.log').stat().st_mtime,
    'limit': 'All local checks scoped to Python/read-only patch checks; no Cargo, installs or network. Requested check window approximately ten minutes.',
}
report = {
    'head': head, 'base': BASE,
    'branch': 'codex/triage-area-unknown-diagnostics', 'clean': True,
    'implementation_model': 'Actual inherited OpenAI GPT-6 Astra xhigh; honest inline markers and commit trailer. No independent model review called.',
    'scope': {'files': ['scripts/triage-area.py', 'scripts/tests/test_triage_area.py'], 'insertions': 135, 'deletions': 3},
    'behavior': [
        'An early title-only triage-area rule routes classifier diagnostics to ci before topic rules or T2 crate-token inference. T0 explicit declaration retains precedence.',
        'The unchanged full-plan unknown-label predicate and exit2 remain before budget slicing, --json output or any apply/unpark. Offenders now emit deterministic UNKNOWN_AREA JSON records with issue number, missing label and exact evidence tier/rule.',
        'Remedy text distinguishes wrong routing from separately reviewed provisioning for a verified existing crate. The classifier never creates a label.',
        'Existing area labels are preserved; genuine generator work continues deriving sparq-wrapper-gen. Root-owned exact-label provisioning is external context, not an action of this patch.',
    ],
    'validation': json.loads((ROOT / 'test-counts.json').read_text()),
    'tests_command': '/opt/homebrew/bin/python3 scripts/tests/test_triage_area.py',
    'tests_exit': 0,
    'negative_controls': 'Eight actual production-source mutants compiled and executed against the unchanged seven-test class (56 test executions). Seven produce assertion failures; the newline-unescape control produces a JSONDecodeError in the round-trip test. No compile-only kills or survivors.',
    'preflight': 'Exit1 solely because installed Bash3.2 lacks mapfile in privacy checker line92, followed by BrokenPipeError. G1/G2/G6/guard-untested passed. No-perf-numbers/readme-template correctly skip the two Python files. No Bash>=4 at /opt/homebrew/bin/bash or /usr/local/bin/bash. No workaround, suppression, gate change or install; Linux CI must run the privacy gate.',
    'preflight_input': 'Exact uncommitted whole diff and changed-file list supplied through supported --changed-files/--added-lines options, then verified byte-identical to the committed diff.',
    'composition': 'Read-only git apply --check succeeds for frozen6095 full.diff against this candidate. T0 declaration region and all other production function bodies are byte-identical to base; only main changes. Existing scope-property loops are byte-identical. No5457/6095 change imported, authorship rewritten, branch touched or hold changed.',
    'ci': 'docs-quality.yml has no workflow path filters; its hard scripts/tests job runs triage-area.py --self-test and scripts/tests/test_triage_area.py at625-628. The new tests are part of that actual module. The scheduled classifier self-tests both before --apply. No workflow change needed.',
    'limitations': [
        'No live classifier replay, GitHub reads/writes, dispatch, issue relabel, PR admission or broad board enumeration during implementation.',
        'Mocked CLI tests exercise real main/plan/classify/apply_row but replace candidate fetch, live labels, gh and sleep. The serialization-only fixture explicitly injects a synthetic plan; other tests derive actual T0/T1/T2 records.',
        'The original run did not log row identity;5016 remains a captured candidate, not proven historical input. Current already-labelled6468 is a no-op.',
        'Existing live label list limit500 (#6335), T0 suffix-boundary issue4567/PR6095 and census5457 remain separate. Their behavior was not changed or claimed fixed.',
        'Patch applicability is a mechanical composition check, not combined6095 tests or a new independent review.',
        'Preflight is not fully green locally; the Bash/privacy limitation remains for authoritative Linux validation.',
    ],
    'timing': test_times,
}
(ROOT / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
(ROOT / 'report.md').write_text(
    f'Head `{head}` is a clean, two-file Astra-authored fix (+135/-3). '
    'It adds the title-only classifier rule and escaped row/label/tier diagnostics before the unchanged whole-plan zero-write barrier.\n\n'
    'Validation:44 classifier tests,17 classifier self-test assertions and114 migration assertions pass. '
    'All8 compiled/executed controls are killed. The frozen6095 patch applies cleanly; T0 and existing property loops remain unchanged.\n\n'
    'Author preflight fails only at the existing Bash3 privacy/mapfile limitation. No gate was weakened and no install, network replay, Rust build or remote mutation was performed. '
    'Linux privacy validation and actual independent Opus review remain required.\n'
)

def source_selection(relative, functions=(), assignments=()):
    source = (WT / relative).read_text()
    tree = ast.parse(source)
    chunks = []
    for node in tree.body:
        name = node.name if isinstance(node, (ast.FunctionDef, ast.ClassDef)) else None
        targets = [t.id for t in node.targets if isinstance(t, ast.Name)] if isinstance(node, ast.Assign) else []
        if name in functions or set(targets).intersection(assignments):
            chunks.append(f'# {relative}:{node.lineno}\n' + ast.get_source_segment(source, node))
    return '\n\n'.join(chunks) + '\n'

critical = source_selection('scripts/triage-area.py',
    functions=('declared_areas', 'classify', '_bd_derive', '_gh', 'crate_names',
               'live_area_labels', 'label_names', 'open_issues', 'candidate_issues',
               'plan', 'apply_row', '_positive_int', '_non_negative_float', 'main'),
    assignments=('RULES', '_DECL', '_DECL_PATHS', '_SURFACE_TOKENS', 'REPO', 'PARK_LABEL',
                 'MAX_WRITES_PER_RUN', 'WRITE_PACE_SECONDS', 'FETCH_CEILING'))
(ROOT / 'critical-source.py.txt').write_text(critical)
fallback = source_selection('scripts/bd-to-issues.py',
    functions=('_scope_to_area', '_token_hits', 'derive_areas'),
    assignments=('_TITLE_SCOPE', '_SURFACE_AREAS'))
(ROOT / 'fallback-source.py.txt').write_text(fallback)
tests = source_selection('scripts/tests/test_triage_area.py',
    functions=('TestTriageAreaDiagnostics', '_load', 'areas', 'evidence'),
    assignments=('REPO_ROOT', 'TA', 'CRATES'))
(ROOT / 'focused-tests.py.txt').write_text(tests)
ci = (WT / '.github/workflows/docs-quality.yml').read_text().splitlines()
ci_excerpt = '\n'.join(f'{i}: {line}' for i, line in enumerate(ci, 1)
                       if i <= 45 or 595 <= i <= 630)
(ROOT / 'ci-wiring.txt').write_text(ci_excerpt + '\n')
parts = [
    ('Report', 'report.json', 'json'), ('Commit', 'commit.txt', 'text'),
    ('Whole two-file diff', 'full.diff', 'diff'),
    ('Critical actual classifier, I/O and CLI source', 'critical-source.py.txt', 'python'),
    ('Complete T2 derivation and scope constants', 'fallback-source.py.txt', 'python'),
    ('Actual test module loading and complete new tests', 'focused-tests.py.txt', 'python'),
    ('Actual CI wiring', 'ci-wiring.txt', 'yaml'),
    ('Composition and preservation', 'scope-composition.json', 'json'),
    ('Full classifier suite result', 'triage-area-tests.log', 'text'),
    ('Classifier self-test result', 'triage-self-test-exact-head.log', 'text'),
    ('Self-test commands', 'self-test-commands.json', 'json'),
    ('Actual preflight command', 'preflight-result.json', 'json'),
    ('Actual preflight failure', 'preflight.log', 'text'),
    ('Negative control runner', 'run-controls.py', 'python'),
    ('All calibrated control outcomes', 'controls.json', 'json'),
]
packet = f'# #6468 focused implementation review\n\nExact head `{head}` on `{BASE}`.\n\n'
for title, path, language in parts:
    packet += f'## {title}\n\n`{path}`\n\n```{language}\n' + (ROOT / path).read_text() + '\n```\n\n'
# Keep the actual failed-control bodies available in the immutable manifest;
# include each exact mutation hunk here so the reviewer can calibrate the result.
for folder in sorted((ROOT / 'controls').iterdir()):
    packet += f'## Control {folder.name}\n\n```diff\n' + (folder / 'mutation.diff').read_text() + '\n```\n\n'
(ROOT / 'review-packet.md').write_text(packet)
entries = []
for path in sorted(ROOT.rglob('*')):
    if path.is_file() and path.name not in ('manifest.json', 'freeze.json'):
        entries.append({'path': str(path.relative_to(ROOT)), 'bytes': path.stat().st_size,
                        'sha256': sha(path.read_bytes())})
(ROOT / 'manifest.json').write_text(json.dumps({'head': head, 'files': entries}, indent=2) + '\n')
freeze = {'head': head, 'base': BASE, 'clean': True,
          'packet_path': str(ROOT / 'review-packet.md'),
          'packet_bytes': (ROOT / 'review-packet.md').stat().st_size,
          'packet_sha256': sha((ROOT / 'review-packet.md').read_bytes()),
          'manifest_files': len(entries),
          'manifest_sha256': sha((ROOT / 'manifest.json').read_bytes())}
(ROOT / 'freeze.json').write_text(json.dumps(freeze, indent=2) + '\n')
print(json.dumps(freeze, indent=2))
