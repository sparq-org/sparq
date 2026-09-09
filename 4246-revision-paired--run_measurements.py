from pathlib import Path
import subprocess, time, json, hashlib, os
p=Path(__file__).resolve().parents[1]
sequence=['main-time','candidate-time','candidate-count','main-count']
records=[]
for name in sequence:
 binary=p/'binaries'/name
 begin=time.time()
 with (p/'paired'/(name+'.jsonl')).open('w') as out, (p/'paired'/(name+'.stderr')).open('w') as err:
  r=subprocess.run([str(binary),'lifecycle'],stdout=out,stderr=err,env={**os.environ,'RAYON_NUM_THREADS':'1'},timeout=300)
 records.append(dict(name=name,exit=r.returncode,started=begin,ended=time.time(),binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest()))
 (p/'paired/execution.json').write_text(json.dumps(records,indent=2)+'\n')
 print(records[-1],flush=True)
 assert r.returncode==0
