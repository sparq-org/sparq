from pathlib import Path
import subprocess,json,time,shutil,os,hashlib
p=Path(__file__).resolve().parent;(p/'raw').mkdir();samples=[];receipts=[]
try:
 for case in ['first','miss']:
  for rep in range(5):
   for source in (['baseline','candidate'] if rep%2==0 else ['candidate','baseline']):
    assert shutil.disk_usage(p).free>6509559808
    name=f'{case}-{rep}-{source}';binary=p/'binary'/source;argv=[str(binary),case,'base',str(rep)]
    start=time.monotonic();r=subprocess.run(argv,capture_output=True,text=True,timeout=45,env={**os.environ,'RAYON_NUM_THREADS':'1'})
    (p/'raw'/(name+'.stdout')).write_text(r.stdout);(p/'raw'/(name+'.stderr')).write_text(r.stderr)
    receipt=dict(argv=argv,exit=r.returncode,seconds=time.monotonic()-start,binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),order=len(receipts),free_bytes=shutil.disk_usage(p).free);receipts.append(receipt)
    assert r.returncode==0,name
    rows=[json.loads(line) for line in r.stdout.splitlines() if line.strip()];data=[x for x in rows if 'nanos' in x];assert len(data)==1
    d=data[0];assert d['rows']==(3 if case=='first' else 0) and d['case']==case and not d['counting'] and d['iterations']==3
    d.update(source=source,fixture=rows[0],execution_order=receipt['order']);samples.append(d)
  print(case,'complete',len(samples),'samples',flush=True)
finally:
 (p/'samples.json').write_text(json.dumps(samples,indent=2)+'\n')
 (p/'commands.json').write_text(json.dumps(receipts,indent=2)+'\n')
