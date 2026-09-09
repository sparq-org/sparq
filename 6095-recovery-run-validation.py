# [GPT-6 Astra] Hermetic recovery validation; no live classifier calls.
from pathlib import Path
import subprocess, os, json, time, sys
E=Path(__file__).resolve().parent
W=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/pr6095')
env=dict(os.environ, PATH='/opt/homebrew/bin:'+os.environ['PATH'], PYTHONDONTWRITEBYTECODE='1')
commands=[
 ('python-version',[sys.executable,'--version']),
 ('classifier-suite',[sys.executable,'scripts/tests/test_triage_area.py']),
 ('classifier-selftest',[sys.executable,'scripts/triage-area.py','--self-test']),
 ('migration-selftest',[sys.executable,'scripts/bd-to-issues.py','--self-test']),
 ('author-preflight',[sys.executable,'scripts/preflight.py','--changed-files',str(E/'changed-files.txt'),'--added-lines',str(E/'full.diff')]),
 ('diff-check',['git','diff','--check','e53464c73f31f7aca800f3867ac054c36408e346']),
 ('bash-version',['/bin/bash','--version']),
]
rows=[]
for name,cmd in commands:
 t=time.monotonic()
 r=subprocess.run(cmd,cwd=W,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=180)
 (E/(name+'.log')).write_bytes(r.stdout)
 rows.append(dict(name=name,argv=cmd,cwd=str(W),exit=r.returncode,elapsed_seconds=time.monotonic()-t,log=name+'.log'))
 print(name,r.returncode,flush=True)
(E/'commands.json').write_text(json.dumps(rows,indent=2)+'\n')
