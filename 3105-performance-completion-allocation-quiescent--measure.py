from pathlib import Path
import subprocess,time,json,hashlib,shutil,os,sys,datetime
e=Path(__file__).resolve().parent
threshold=6*1024**3+64*1024**2;started=time.monotonic();samples=[];receipts=[]
assert all((e/'binary'/name).exists() for name in ['baseline-count','candidate-count'])
(e/'raw').mkdir(exist_ok=True)
try:
 for mode,reps in [('count',3)]:
  for case in ['first','second','miss','late','multi']:
   for view in ['base','overlay']:
    for rep in range(reps):
     order=['baseline','candidate'] if rep%2==0 else ['candidate','baseline']
     for source in order:
      assert shutil.disk_usage(e).free>threshold,'disk reserve before sample'
      assert time.monotonic()-started<600,'whole measurement cap'
      name=f'{mode}-{case}-{view}-{rep}-{source}';binary=e/'binary'/(source+'-'+mode)
      argv=[str(binary),case,view,str(rep)];begin=time.monotonic()
      run=subprocess.run(argv,capture_output=True,text=True,timeout=45,env={**os.environ,'RAYON_NUM_THREADS':'1'})
      assert shutil.disk_usage(e).free>threshold,'disk reserve before evidence write'
      (e/'raw'/(name+'.stdout')).write_text(run.stdout);(e/'raw'/(name+'.stderr')).write_text(run.stderr)
      receipt={'name':name,'argv':argv,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'exit':run.returncode,'seconds':time.monotonic()-begin,'order':len(receipts),'free_bytes':shutil.disk_usage(e).free}
      receipts.append(receipt)
      if run.returncode:raise RuntimeError('sample failed: '+name)
      rows=[json.loads(line) for line in run.stdout.splitlines() if line.strip()]
      fixture=[x for x in rows if x.get('kind')=='fixture'];data=[x for x in rows if 'nanos' in x]
      assert len(fixture)==1 and len(data)==1
      d=data[0];assert d['case']==case and d['view']==view and d['rep']==rep and d['counting']==(mode=='count')
      d.update(source=source,mode=mode,fixture=fixture[0],binary_sha256=receipt['binary_sha256'],execution_order=receipt['order'])
      samples.append(d)
    print(mode,case,view,'complete',len(samples),'samples',round(time.monotonic()-started),'seconds',flush=True)
finally:
 assert shutil.disk_usage(e).free>6*1024**3
 (e/'samples.json').write_text(json.dumps(samples,indent=2)+'\n')
 (e/'measurement-commands.json').write_text(json.dumps(receipts,indent=2)+'\n')
 (e/'measurement-finish.json').write_text(json.dumps({'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'seconds':time.monotonic()-started,'samples':len(samples),'commands':len(receipts),'free_bytes':shutil.disk_usage(e).free},indent=2)+'\n')
