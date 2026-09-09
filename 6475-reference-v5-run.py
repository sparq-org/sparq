# [GPT-6 Astra] Bounded isolated execution; no install or lifecycle scripts.
from pathlib import Path
import json,subprocess,shutil,time
E=Path(__file__).resolve().parent
node=Path('/Users/jesght/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node')
records=[]
for name in ['reference-replay','reference-fixtures']:
 before=shutil.disk_usage(E).free
 assert before>=6*1024**3
 argv=[str(node),name+'.cjs'];start=time.monotonic()
 p=subprocess.run(argv,cwd=E,capture_output=True,timeout=15)
 (E/(name+'.json')).write_bytes(p.stdout);(E/(name+'.stderr.log')).write_bytes(p.stderr)
 record=dict(argv=argv,exit=p.returncode,seconds=time.monotonic()-start,free_before=before,free_after=shutil.disk_usage(E).free)
 records.append(record);(E/'commands.json').write_text(json.dumps(records,indent=2)+'\n');print(record)
 assert p.returncode==0 and record['free_after']>=6*1024**3
