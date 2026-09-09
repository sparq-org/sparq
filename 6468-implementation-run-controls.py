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
results = []
for name, before, after in mutations:
    assert time.monotonic() - started < 120, 'bounded controls exceeded 120 seconds'
    assert source.count(before) == 1, name
    candidate = source.replace(before, after)
    folder = ROOT / 'controls' / name
    folder.mkdir(parents=True)
    (folder / 'triage-area.py').write_text(candidate)
    (folder / 'mutation.diff').write_text(''.join(difflib.unified_diff(
        source.splitlines(True), candidate.splitlines(True),
        fromfile='candidate/triage-area.py', tofile=name + '/triage-area.py')))
    compiled = compile(candidate, str(folder / 'triage-area.py'), 'exec')
    module = types.ModuleType('triage_area_mutant')
    module.__file__ = str(SOURCE)
    exec(compiled, module.__dict__)
    # Load the actual unchanged test file, then substitute only the compiled
    # production module. Imports and crate discovery still use the real checkout.
    spec = importlib.util.spec_from_file_location('triage_area_control_tests', WT / 'scripts/tests/test_triage_area.py')
    tests = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(tests)
    tests.TA = module
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(tests.TestTriageAreaDiagnostics)
    output = io.StringIO()
    with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
        result = unittest.TextTestRunner(stream=output, verbosity=2).run(suite)
    (folder / 'test.log').write_text(output.getvalue())
    record = dict(name=name, compiled=True, tests=result.testsRun,
                  failures=len(result.failures), errors=len(result.errors),
                  killed=not result.wasSuccessful(),
                  failed_tests=[test.id() for test, _ in result.failures + result.errors],
                  source_sha256=hashlib.sha256(candidate.encode()).hexdigest())
    results.append(record)
    print(json.dumps(record), flush=True)
summary = dict(candidate_sha256=hashlib.sha256(source.encode()).hexdigest(),
               tests_identical_to_worktree=True, elapsed_seconds=time.monotonic() - started,
               controls=results, survivors=[r['name'] for r in results if not r['killed']])
(ROOT / 'controls.json').write_text(json.dumps(summary, indent=2) + '\n')
assert not summary['survivors'], summary['survivors']
