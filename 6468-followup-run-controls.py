from pathlib import Path
import contextlib
import difflib
import hashlib
import importlib.util
import io
import json
import sys
import time
import types
import unittest

ROOT = Path(__file__).resolve().parent
WT = Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue6468')
SOURCE = WT / 'scripts/triage-area.py'
source = SOURCE.read_text()
test_path = WT / 'scripts/tests/test_triage_area.py'
test_source = test_path.read_text()
started = time.monotonic()
rule = ('    ("triage-area", "title", r"^triage-area(?:\\.py)?(?:\\s|:|$)",\n'
        '     ["ci"], "scripts/triage-area.py and its workflow"),\n')
mutations = [
    ('delete-rule', rule, ''),
    ('drop-title-anchor', 'r"^triage-area(?:', 'r"triage-area(?:'),
    ('disable-global-guard', '    if unknown:\n', '    if False and unknown:\n'),
    ('guard-only-writable-prefix', 'for _, add, _ in rows for lb in add} - known)',
     'for _, add, _ in rows[:a.max_writes] for lb in add} - known)'),
    ('wrong-row-number', '{"number": it["number"], "label": label, "evidence": why}',
     '{"number": 0, "label": label, "evidence": why}'),
    ('cross-associate-labels', 'for label in sorted(set(add).intersection(unknown)):',
     'for label in unknown:'),
    ('drop-tier-evidence', '"label": label, "evidence": why}',
     '"label": label, "evidence": ""}'),
    ('unescape-record-newlines', 'sort_keys=True), file=sys.stderr)',
     'sort_keys=True).replace(chr(92) + "n", chr(10)), file=sys.stderr)'),
]
mutations += [('move-rule-late', rule, ''), ('remove-anchor-metadata', '', '')]
results = []
for name, before, after in mutations:
    assert time.monotonic() - started < 120, 'bounded controls exceeded 120 seconds'
    if name == 'remove-anchor-metadata':
        candidate = source
        metadata = '"site-page", "deploy-demo", "triage-area"}'
        assert test_source.count(metadata) == 1
        selected_tests = test_source.replace(metadata, '"site-page", "deploy-demo"}')
    else:
        assert source.count(before) == 1, name
        candidate = source.replace(before, after)
        selected_tests = test_source
    if name == 'move-rule-late':
        closing = candidate.index('\n]\n', candidate.index('RULES = ['))
        candidate = candidate[:closing] + '\n' + rule.rstrip('\n') + candidate[closing:]
    folder = ROOT / 'controls' / name
    folder.mkdir(parents=True)
    (folder / 'triage-area.py').write_text(candidate)
    (folder / 'mutation.diff').write_text(''.join(difflib.unified_diff(
        source.splitlines(True), candidate.splitlines(True),
        fromfile='candidate/triage-area.py', tofile=name + '/triage-area.py')))
    (folder / 'tests.py').write_text(selected_tests)
    if name == 'remove-anchor-metadata':
        (folder / 'test-metadata.diff').write_text(''.join(difflib.unified_diff(
            test_source.splitlines(True), selected_tests.splitlines(True),
            fromfile='candidate/tests.py', tofile='control/tests.py')))
    compiled = compile(candidate, str(folder / 'triage-area.py'), 'exec')
    module = types.ModuleType('triage_area_mutant')
    module.__file__ = str(SOURCE)
    exec(compiled, module.__dict__)
    # Load the actual unchanged test file, then substitute only the compiled
    # production module. Imports and crate discovery still use the real checkout.
    spec = importlib.util.spec_from_file_location('triage_area_control_tests', WT / 'scripts/tests/test_triage_area.py')
    tests = importlib.util.module_from_spec(spec)
    exec(compile(selected_tests, str(test_path), 'exec'), tests.__dict__)
    tests.TA = module
    full_suite = name in ('drop-title-anchor', 'remove-anchor-metadata')
    suite = (unittest.defaultTestLoader.loadTestsFromModule(tests) if full_suite else
             unittest.defaultTestLoader.loadTestsFromTestCase(tests.TestTriageAreaDiagnostics))
    output = io.StringIO()
    with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
        result = unittest.TextTestRunner(stream=output, verbosity=2).run(suite)
    (folder / 'test.log').write_text(output.getvalue())
    record = dict(name=name, compiled=True, full_suite=full_suite,
                  metadata_control=name == 'remove-anchor-metadata', tests=result.testsRun,
                  failures=len(result.failures), errors=len(result.errors),
                  killed=not result.wasSuccessful(),
                  failed_tests=[test.id() for test, _ in result.failures + result.errors],
                  source_sha256=hashlib.sha256(candidate.encode()).hexdigest())
    results.append(record)
    print(json.dumps(record), flush=True)
summary = dict(candidate_sha256=hashlib.sha256(source.encode()).hexdigest(),
               tests_identical_to_worktree_except_explicit_metadata_control=True, elapsed_seconds=time.monotonic() - started,
               controls=results, survivors=[r['name'] for r in results if not r['killed']])
(ROOT / 'controls.json').write_text(json.dumps(summary, indent=2) + '\n')
assert not summary['survivors'], summary['survivors']
