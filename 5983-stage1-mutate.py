"""[GPT-6 Astra] Bounded local guard controls; restore the exact source in finally."""
import hashlib,json,os,re,subprocess
from pathlib import Path
root=Path.cwd();out=Path(__file__).parent;source=root/'crates/sparq-engine/src/exec.rs'
original=source.read_text();assert subprocess.run(['git','diff','--quiet'],cwd=root).returncode==0
repeat='if patterns.iter().any(has_intra_triple_repeated_var) {\n        return Ok(None);\n    }\n    // Out of scope'
budget='if budget::active() {\n        return Ok(None);\n    }\n    if view::default_is_empty()'
fanout='if match_count != 1 {\n                    return Ok(None);\n                }'
mutants=[('repeated_deleted',repeat,'// Out of scope'),('repeated_inert',repeat,repeat.replace('if patterns','if false && patterns')),('budget_deleted',budget,'if view::default_is_empty()'),('budget_inert',budget,budget.replace('if budget','if false && budget')),('fanout_guard_deleted',fanout,''),('indexed_path_disabled','let Some((desc, order_var)) = expression.first()', 'if true { return Ok(None); }\n    let Some((desc, order_var)) = expression.first()')]
env=dict(os.environ,CARGO_BUILD_JOBS='2',CARGO_TARGET_DIR=str(out/'target'),CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
command=['/Users/jesght/.cargo/bin/cargo','test','--locked','--offline','-p','sparq-engine','--test','topk_orderby_indexed_differential','--','--test-threads=1']
def execute(name):
 result=subprocess.run(command,cwd=root,env=env,capture_output=True,text=True,timeout=120)
 text=result.stdout+result.stderr;(out/f'mutant-{name}.txt').write_text(text)
 match=re.search(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed;',text)
 passed=int(match[2]) if match else None;failed=int(match[3]) if match else None
 count=passed+failed if match else None
 classification=('PASS' if name in ['baseline','restored'] else 'SURVIVED') if result.returncode==0 and count==19 else ('KILLED' if result.returncode==101 and count==19 and failed else 'INVALID_OR_ERROR')
 record=dict(name=name,exit_code=result.returncode,tests=count,passed=passed,failed=failed,classification=classification,source_sha256=hashlib.sha256(source.read_bytes()).hexdigest());print(json.dumps(record),flush=True);return record
results=[]
try:
 baseline=execute('baseline');assert baseline['classification']=='PASS'
 for name,old,new in mutants:
  assert original.count(old)==1,(name,original.count(old))
  source.write_text(original.replace(old,new,1));results.append(execute(name))
finally:
 source.write_text(original)
restored=execute('restored');assert restored['classification']=='PASS'
assert subprocess.run(['git','diff','--quiet'],cwd=root).returncode==0
report=dict(baseline=baseline,mutants=results,restored=restored,counts={key:sum(r['classification']==key for r in results) for key in ['KILLED','SURVIVED','INVALID_OR_ERROR']})
(out/'mutation-report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report['counts']),flush=True)
