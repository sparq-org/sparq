# GPT-6 Astra: executed mutations of the exact module, no production edits.
from pathlib import Path
import subprocess,json,difflib
v=Path(__file__).resolve().parent;base=(v/'actual-budget-final.rs').read_text();template=json.loads((v/'compile-module-final.json').read_text())['command'];results=[]
def run(label,cmd):
 r=subprocess.run(['python3',str(v/'run.py'),label,*cmd],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
 print(r.stdout.decode());return json.loads((v/(label+'.json')).read_text())
for label,needle,replacement in [
 ('final-omit-limits','ACTIVE.with(|a| a.set(self.previous));','ACTIVE.with(|a| a.set(OFF));'),
 ('final-omit-sticky','EXCEEDED.with(|e| e.set(self.exceeded));','EXCEEDED.with(|e| e.set(None));'),
 ('final-omit-bytes','ACTIVE.with(|a| a.set(self.previous));','ACTIVE.with(|a| a.set(Limits { byte_width: BYTES_PER_ID, extra_bytes: 0, ..self.previous }));')]:
 assert base.count(needle)==1;mutant=base.replace(needle,replacement);source=v/(label+'.rs');source.write_text(mutant);(v/(label+'.diff')).write_text(''.join(difflib.unified_diff(base.splitlines(True),mutant.splitlines(True),fromfile='actual-budget-final.rs',tofile=source.name)))
 cmd=[str(source) if x==str(v/'actual-budget-final.rs') else str(v/'binary'/label) if x==str(v/'binary/actual-budget-final') else x for x in template]
 c=run('compile-'+label,cmd);assert c['exit_code']==0 and c['stop'] is None,c
 t=run('test-'+label,[str(v/'binary'/label),'exec::budget::nested_budget_tests::','--test-threads=1']);assert t['exit_code']==101 and t['stop'] is None,t
 results.append({'control':label,'compile_exit':c['exit_code'],'test_exit':t['exit_code'],'executed':True})
sibling='''\nmod sibling {\n    pub fn escaped(b: &crate::QueryBudget) -> crate::exec::budget::Guard<'_> {\n        crate::exec::budget::install(b)\n    }\n    pub fn detached() -> crate::exec::budget::Limits { crate::exec::budget::snapshot() }\n}\n'''
sealed=base+sibling
for label,text,expected in [('final-sealed',sealed,1),('final-opened-control',sealed.replace('    struct Guard<','    pub(crate) struct Guard<').replace('    fn install(','    pub(crate) fn install(').replace('pub(in crate::exec)','pub(crate)'),0)]:
 source=v/(label+'.rs');source.write_text(text);cmd=[]
 for x in template:
  if x=='--test':cmd.extend(['--crate-type','lib','--emit','metadata'])
  elif x==str(v/'actual-budget-final.rs'):cmd.append(str(source))
  elif x==str(v/'binary/actual-budget-final'):cmd.append(str(v/(label+'.rmeta')))
  else:cmd.append(x)
 r=run('compile-'+label,cmd);assert r['exit_code']==expected and r['stop'] is None,r;results.append({'control':label,'compile_exit':r['exit_code'],'executed_runtime':False})
 if label=='final-sealed':assert 'E0603' in (v/'compile-sealed.log').read_text()
(v/'controls-final-summary.json').write_text(json.dumps(results,indent=2)+'\n')
