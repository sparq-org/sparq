import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue6462')
OUT = Path(__file__).resolve().parent
TEST = 'TestCancelledActionsRecovery.test_distinct_check_job_and_run_ids_reach_actions_only'
spec = importlib.util.spec_from_file_location('schema_control_tests', ROOT / 'scripts/tests/test_arm_capability_wiring.py')
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)
source = (ROOT / 'scripts/rearm-sweeper.py').read_text()
fixed = 'author{login __typename ... on Bot{id}}'
old = 'author{id login __typename}'
assert source.count(fixed) == 1
results = {}
with tempfile.TemporaryDirectory(dir=OUT) as directory:
    normalization = 'actor_login = bot["login"].removesuffix("[bot]")'
    assert source.count(normalization) == 1
    cases = [('fixed_query', source), ('old_query', source.replace(fixed, old, 1)),
             ('old_login', source.replace(normalization, 'actor_login = bot["login"]', 1))]
    for label, code in cases:
        path = Path(directory) / 'rearm-sweeper.py'
        path.write_text(code)
        module.REARM_PY = path
        stream = io.StringIO()
        suite = unittest.defaultTestLoader.loadTestsFromName(TEST, module)
        result = unittest.TextTestRunner(stream=stream, verbosity=2).run(suite)
        (OUT / f'6462-final-control-{label}.txt').write_text(stream.getvalue())
        results[label] = dict(tests=result.testsRun, failures=len(result.failures),
                             errors=len(result.errors), skipped=len(result.skipped),
                             source_sha256=hashlib.sha256(code.encode()).hexdigest())
assert all(result['tests'] == 1 for result in results.values())
assert results['fixed_query']['failures'] == results['fixed_query']['errors'] == 0
assert results['old_query']['failures'] == 1 and results['old_query']['errors'] == 0
assert results['old_login']['failures'] == 1 and results['old_login']['errors'] == 0
assert all(result['skipped'] == 0 for result in results.values())
(OUT / '6462-final-control-results.json').write_text(json.dumps(results, indent=2) + '\n')
print(json.dumps(results, indent=2))
