# [GPT-6 Astra] Exact composition and coverage evidence, with no source mutation.
from pathlib import Path
import ast, hashlib, json, subprocess, sys
E=Path(__file__).resolve().parent; W=Path.cwd()
BASE='e53464c73f31f7aca800f3867ac054c36408e346'; OLD='0883db681c73ddc338643f135c770c5a036ac3a4'
def git(*args): return subprocess.check_output(['git',*args])
def patchid(data): return subprocess.check_output(['git','patch-id','--stable'],input=data).decode().split()[0]
old=git('diff',OLD+'^',OLD); new=git('diff',BASE)
(E/'historical.diff').write_bytes(old)
assert patchid(old)==patchid(new)
changed=git('diff','--name-only',BASE).decode().splitlines()
assert changed==['scripts/tests/test_triage_area.py','scripts/triage-area.py'],changed

def stmts(src):
 tree=ast.parse(src)
 return {getattr(n,'name',None) or (','.join(t.id for t in n.targets if isinstance(t,ast.Name)) if isinstance(n,ast.Assign) else ''):ast.dump(n,include_attributes=False) for n in tree.body}
pro='scripts/triage-area.py'; tests='scripts/tests/test_triage_area.py'
m=stmts(git('show',BASE+':'+pro)); c=stmts((W/pro).read_text())
assert {k:v for k,v in m.items() if k!='_DECL'}=={k:v for k,v in c.items() if k!='_DECL'}
mt=stmts(git('show',BASE+':'+tests)); ct=stmts((W/tests).read_text())
assert mt['TestTriageAreaDiagnostics']==ct['TestTriageAreaDiagnostics']
class_before=next(n for n in ast.parse(git('show',BASE+':'+tests)).body if isinstance(n,ast.ClassDef) and n.name=='TestScopeDiscipline')
class_after=next(n for n in ast.parse((W/tests).read_text()).body if isinstance(n,ast.ClassDef) and n.name=='TestScopeDiscipline')
assert ast.dump(class_before.body[0])==ast.dump(class_after.body[0]) # first expression/docstring
anch=lambda cls:next(n for n in cls.body if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='ANCHORED_ONLY' for t in n.targets))
assert ast.dump(anch(class_before))==ast.dump(anch(class_after))
coverage=json.loads((E/'coverage.json').read_text())
spans=[]
for name,data in coverage['files'].items():
 src=(W/name).read_text(); lines=src.splitlines(); hit=data['executed_line_counts']
 for node in ast.walk(ast.parse(src)):
  if isinstance(node,ast.FunctionDef) and node.name in data['critical_function_spans']:
   executable=set()
   for sub in ast.walk(node):
    if isinstance(sub,ast.stmt) and not isinstance(sub,(ast.FunctionDef,ast.AsyncFunctionDef)) and not (isinstance(sub,ast.Expr) and isinstance(sub.value,ast.Constant) and isinstance(sub.value.value,str)):
     executable.add(sub.lineno)
   missing=sorted(n for n in executable if str(n) not in hit)
   spans.append(dict(file=name,function=node.name,statement_lines=len(executable),never_executed_statement_start_lines=[dict(line=n,source=lines[n-1]) for n in missing]))
(E/'coverage-summary.json').write_text(json.dumps(dict(method='stdlib trace line events; executable statement start lines from AST, excluding docstrings; not branch coverage',critical_functions=spans,calibration=coverage['calibration']),indent=2)+'\n')
report=dict(base=BASE,old_head=OLD,old_parent=git('rev-parse',OLD+'^').decode().strip(),historical_patch_id=patchid(old),net_patch_id=patchid(new),net_files=changed,production_all_top_level_AST_except_DECL_identical=True,main_TestTriageAreaDiagnostics_AST_identical=True,ANCHORED_ONLY_AST_identical=True,no_conflict_resolution_required=True,source_files={f:hashlib.sha256((W/f).read_bytes()).hexdigest() for f in changed},python=sys.version,workflow_files_identical_to_main=True)
assert not git('diff','--name-only',BASE,'--','.github/workflows')
(E/'composition.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
