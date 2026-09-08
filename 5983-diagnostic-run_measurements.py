"""[GPT-6 Astra] One bounded sequential local matrix; no network or source mutation."""
from pathlib import Path
import subprocess,os,json,time,hashlib
out=Path(__file__).parent;raw=out/'raw';raw.mkdir(exist_ok=True)
cases=[('base',1),('base',512),('overlay',1),('overlay',512),('drained',1),('drained-overlay',1),('ties',1),('ties-overlay',1)]
variants=['candidate','disabled','main'];records=[]
for binary in sorted((out/'bins').iterdir()):assert binary.is_file()
# Round-robin source order by fixture avoids running every candidate sample first.
for index,(case,k) in enumerate(cases):
 for variant in variants[index%3:]+variants[:index%3]:
  expected='indexed' if variant=='candidate' and case in ['base','overlay'] else 'fallback'
  for mode in ['verify','timing','count']:
   binary=out/'bins'/(variant+'-'+('count' if mode=='count' else 'timing'))
   command=[str(binary),'verify' if mode=='verify' else 'measure',case,'50000',str(k),'0' if mode=='verify' else '2','1' if mode=='verify' else ('3' if mode=='count' else '7'),expected]
   name=f'{case}-k{k}-{variant}-{mode}';before=time.time();load=os.getloadavg()
   result=subprocess.run(command,capture_output=True,text=True,timeout=120,env=dict(os.environ,RAYON_NUM_THREADS='1'))
   (raw/(name+'.txt')).write_text(result.stdout+result.stderr)
   parsed=[]
   for line in result.stdout.splitlines():
    if not line.startswith('kind='):continue
    row={}
    for field in line.split('\t'):
     key,value=field.split('=',1)
     try:value=int(value)
     except ValueError:pass
     row[key]=value
    parsed.append(row)
   record=dict(name=name,variant=variant,case=case,n=50000,k=k,mode=mode,command=command,binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),exit_code=result.returncode,wall_seconds=time.time()-before,started_unix=before,load_average=load,rows=parsed,raw=str(raw/(name+'.txt')))
   records.append(record);(out/'measurements.json').write_text(json.dumps(records,indent=2)+'\n');print(json.dumps({key:record[key] for key in ['name','exit_code','wall_seconds']}),flush=True)
   if result.returncode:raise SystemExit('Stop on failing fixture/harness; no automatic retry')
print(json.dumps({'completed':len(records),'query_samples':sum(sum(x['kind']=='sample' for x in r['rows']) for r in records)}),flush=True)
