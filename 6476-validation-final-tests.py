from pathlib import Path
import json,subprocess,shutil,hashlib
v=Path(__file__).resolve().parent;cache=Path('/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/release');shutil.copy2(cache/'nested-budget-validation',v/'binary/public-final')
def run(label,cmd):
 r=subprocess.run(['python3',str(v/'run.py'),label,*cmd],stdout=subprocess.PIPE,stderr=subprocess.STDOUT);print(r.stdout.decode());j=json.loads((v/(label+'.json')).read_text());assert j['exit_code']==0 and j['stop'] is None,j
run('public-final',[str(v/'binary/public-final'),'--require-restoration'])
a=json.loads((v/'compile-module.json').read_text())['command'];a=[x.replace('actual-budget.rs','actual-budget-final.rs').replace('/binary/actual-budget','/binary/actual-budget-final') for x in a]
run('compile-module-final',a);run('tests-module-final',[str(v/'binary/actual-budget-final'),'--test-threads=1'])
a[0]=a[0].replace('/rustc','/clippy-driver');a=[str(v/'clippy-tests-final.rmeta') if x==str(v/'binary/actual-budget-final') else x for x in a];a+=['--emit=metadata','-D','warnings'];run('clippy-tests-final',a)
s=(v/'controls.py').read_text().replace('actual-budget.rs','actual-budget-final.rs').replace('compile-module.json','compile-module-final.json').replace('binary/actual-budget','binary/actual-budget-final').replace('controls-summary.json','controls-final-summary.json')
for label in ['omit-limits','omit-sticky','omit-bytes','sealed','opened-control']:s=s.replace("'"+label+"'","'final-"+label+"'")
# Template output substitution uses v/'binary'/label, unchanged and unique.
(v/'controls-final.py').write_text(s)
r=subprocess.run(['python3',str(v/'controls-final.py')],stdout=subprocess.PIPE,stderr=subprocess.STDOUT);print(r.stdout.decode());assert r.returncode==0
