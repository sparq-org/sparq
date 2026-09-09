# [GPT-6 Astra] Compile and execute exact merged modules with bounded mutations.
# __file__ remains the genuine source path: crate inventory uses the same checkout.
from pathlib import Path
import ast, difflib, io, json, sys, trace, types, unittest
E=Path(__file__).resolve().parent
W=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/pr6095')
P=W/'scripts/triage-area.py'; T=W/'scripts/tests/test_triage_area.py'
p=P.read_text(); t=T.read_text()

def replace_once(src, old, new):
    assert src.count(old)==1, (old, src.count(old))
    return src.replace(old,new,1)

def run(name, ps, ts):
    log=io.StringIO()
    testmod=types.ModuleType('recovery_tests'); testmod.__file__=str(T)
    exec(compile(ts,str(T),'exec'),testmod.__dict__)
    production=types.ModuleType('triage_area'); production.__file__=str(P)
    sys.modules['triage_area']=production
    exec(compile(ps,str(P),'exec'),production.__dict__)
    testmod.TA=production
    result=unittest.TextTestRunner(stream=log,verbosity=2).run(unittest.defaultTestLoader.loadTestsFromModule(testmod))
    (E/(name+'.log')).write_text(log.getvalue())
    return dict(name=name,tests=result.testsRun,failures=[str(test) for test,_ in result.failures],errors=[str(test) for test,_ in result.errors],success=result.wasSuccessful())

# Coverage first. Calibrate on executable lines in an uncalled function.
instrument=trace.Trace(count=True,trace=False)
baseline=instrument.runfunc(run,'coverage-baseline',p,t)
assert baseline['success'] and baseline['tests']==47,baseline
calibration={}
instrument.runfunc(exec,compile('def never_called():\n    value = 123\n    return value\ndef called():\n    return 456\ncalled()\n','coverage-calibration.py','exec'),calibration)
assert ('coverage-calibration.py',5) in instrument.results().counts
assert not any(fn=='coverage-calibration.py' and ln in (2,3) for fn,ln in instrument.results().counts)
counts=instrument.results().counts
coverage={}
for path,source in [(P,p),(T,t)]:
    byline={str(ln):n for (fn,ln),n in counts.items() if fn==str(path)}
    names={'declared_areas','classify','_assert_the_partition_was_fully_covered',
           'test_a_declaration_shaped_string_in_prose_is_not_a_declaration',
           'test_a_real_declaration_still_reaches_t0',
           'test_a_title_scoped_rule_never_fires_from_the_body_alone',
           'test_a_text_scoped_rule_does_fire_from_the_body'}
    spans={node.name:[node.lineno,node.end_lineno] for node in ast.walk(ast.parse(source)) if isinstance(node,ast.FunctionDef) and node.name in names}
    coverage[str(path.relative_to(W))]=dict(executed_line_counts=byline,critical_function_spans=spans)
(E/'coverage.json').write_text(json.dumps(dict(baseline=baseline,calibration='uncalled function executable lines 2/3 have zero counts',files=coverage),indent=2)+'\n')

raw='r"(?<![\\w-])(?:crate_or_surface|crates?|surface)'
controls=[
 ('remove-left-boundary',replace_once(p,raw,'r"(?:crate_or_surface|crates?|surface)'),t),
 ('word-boundary-hyphen-inert',replace_once(p,raw,'r"\\b(?:crate_or_surface|crates?|surface)'),t),
 ('disable-declaration',replace_once(p,raw,'r"(?!)'+'(?:crate_or_surface|crates?|surface)'),t),
 ('empty-title-partition',p,replace_once(t,'return [r for r in TA.RULES if r[1] == "title"]','return []')),
 ('empty-text-partition',p,replace_once(t,'return [r for r in TA.RULES if r[1] == "text"]','return []')),
 ('partial-title-partition',p,replace_once(t,'return [r for r in TA.RULES if r[1] == "title"]','return [r for r in TA.RULES if r[1] == "title"][:1]')),
]
rows=[]
for name,ps,ts in controls:
    delta=''.join(difflib.unified_diff(p.splitlines(True),ps.splitlines(True),fromfile='a/scripts/triage-area.py',tofile='b/scripts/triage-area.py'))
    delta+=''.join(difflib.unified_diff(t.splitlines(True),ts.splitlines(True),fromfile='a/scripts/tests/test_triage_area.py',tofile='b/scripts/tests/test_triage_area.py'))
    assert delta
    (E/(name+'.diff')).write_text(delta)
    result=run(name,ps,ts)
    result['calibrated_kill']=result['tests']==47 and bool(result['failures']) and not result['errors']
    rows.append(result)
    print(name,result,flush=True)
(E/'controls.json').write_text(json.dumps(rows,indent=2)+'\n')
assert all(r['calibrated_kill'] for r in rows),rows
