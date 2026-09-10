# [GPT-6 Astra] One finite predeclared experiment; no retries/tuning/production writes.
from pathlib import Path
import json,subprocess,shutil,time,itertools,random,collections
E=Path(__file__).resolve().parent;floor=6*1024**3;node='/Users/jesght/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node';start=time.monotonic();records=[];results=[]
def write(p,s):
 b=s.encode() if isinstance(s,str) else s
 assert shutil.disk_usage(E).free>floor+len(b)
 assert sum(f.stat().st_size for f in E.rglob('*') if f.is_file())+len(b)<8*1024**2
 p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(b)
def run(name,text,group,hash='SHA256',control='candidate',expected=None,kind=None):
 assert shutil.disk_usage(E).free>floor
 assert time.monotonic()-start<140
 f=E/'inputs'/(name+'.nq');write(f,text)
 argv=[node,str(E/'case.cjs'),str(f),hash,control];t=time.monotonic()
 try:
  p=subprocess.run(argv,cwd=E,capture_output=True,text=True,timeout=3)
  parsed=json.loads(p.stdout) if p.returncode==0 else {'execution_error':p.stderr}
  rec={'name':name,'argv':argv,'exit':p.returncode,'seconds':time.monotonic()-t,'stdout':p.stdout,'stderr':p.stderr}
 except subprocess.TimeoutExpired:
  parsed={'external_timeout':3};rec={'name':name,'argv':argv,'exit':None,'seconds':time.monotonic()-t,'external_timeout':3}
 records.append(rec);row={'name':name,'group':group,'input':text,'hash':hash,'control':control,'kind':kind,'expected':expected,**parsed};results.append(row)
 if 'result' in parsed and parsed['result']['status']=='error':assert 'output' not in parsed['result'] and 'map' not in parsed['result']
 write(E/'results.json',json.dumps(results,indent=2)+'\n');write(E/'commands.json',json.dumps(records,indent=2)+'\n')
 return row
protocol={'candidate':'enumerate equal-hash top choices and equal-path recursive issuers; compare complete outputs only','max_hndq_calls_across_replays':4000,'max_executions_per_input':64,'total_deadline_ms_per_input':1000,'external_seconds_per_case':3,'whole_run_seconds':140,'corpus':'8 seeded3quad datasets over4 bnodes,2predicates,default/bnode graphs; all6ordersx2 bijective labels','seed':25717,'controls':'first-only on all12 saved variants','suite':'all86 packagedW3C entries','no_retries':True}
write(E/'protocol.json',json.dumps(protocol,indent=2)+'\n')
def variants(prefix,lines,group,controls=False):
 for labeling,rename in [('original',lambda x:x),('renamed',lambda x:x.replace('_:b0','_:x3').replace('_:b1','_:x2').replace('_:b2','_:x1').replace('_:b3','_:x0').replace('_:b4','_:x5').replace('_:b5','_:x4'))]:
  for order in itertools.permutations(range(3)):
   text='\n'.join(rename(lines[i]) for i in order)+'\n';name=prefix+'-'+labeling+'-'+''.join(map(str,order));run(name,text,group)
   if controls:run(name+'-first-only',text,group+'-control',control='first-only')
# Preserve the EXACT previous12-case saved inputs/labels.
prior=json.loads((E.parent/'reference-v5/reference-replay.json').read_text())
for row in prior['all_orders']:
 name='saved-'+row['labeling']+'-'+''.join(map(str,row['order']));run(name,row['input'],'saved');run(name+'-first-only',row['input'],'saved-control',control='first-only')
print('saved matrix complete',flush=True)
F=E.parent/'evidence-completion/fixtures';manifest=json.loads((F/'manifest.jsonld').read_text())
for i,entry in enumerate(manifest['entries']):
 exp=None
 if entry.get('result'):
  s=(F/entry['result']).read_text();exp=json.loads(s) if entry['type']=='rdfc:RDFC10MapTest' else s
 run(entry['id'][1:],(F/entry['action']).read_text(),'W3C',entry.get('hashAlgorithm','SHA256'),expected=exp,kind=entry['type'])
 if i%25==0:print('W3C entries',i+1,flush=True)
# Explicit partial/completion cases, fixed order plus reversal and bijective rename.
saved=(E.parent/'replay/original.nq').read_text();second=saved.replace('_:b','_:d').replace('http://ex/','http://other/')
shapes={'two-distinct-predicate-components':saved+second,'two-same-predicate-components':saved+saved.replace('_:b','_:d'),'saved-plus-cycle':saved+'_:u0 <http://ex/r> _:u1 .\n_:u1 <http://ex/r> _:u0 .\n'}
for name,text in shapes.items():
 for reverse in [False,True]:
  for renamed in [False,True]:
   lines=text.strip().splitlines();lines=lines[::-1] if reverse else lines
   value='\n'.join(lines)+'\n'
   if renamed:value=value.replace('_:b0','_:x8').replace('_:b2','_:x9').replace('_:b3','_:x2').replace('_:b4','_:x0').replace('_:d0','_:y8').replace('_:d2','_:y9').replace('_:d3','_:y2').replace('_:d4','_:y0').replace('_:u0','_:z1').replace('_:u1','_:z0')
   run(name+f'-r{int(reverse)}-n{int(renamed)}',value,name)
rng=random.Random(25717)
for i in range(8):
 tuples=set()
 while len(tuples)<3:tuples.add((rng.randrange(4),rng.randrange(2),rng.randrange(4),rng.randrange(5)))
 lines=[f'_:b{s} <http://ex/p{p}> _:b{o}'+(f' _:b{g}' if g<4 else '')+' .' for s,p,o,g in sorted(tuples)]
 variants(f'corpus{i}',lines,f'corpus{i}')
 print('corpus complete',i,flush=True)
write(E/'finish.json',json.dumps({'seconds':time.monotonic()-start,'cases':len(results),'free_bytes':shutil.disk_usage(E).free},indent=2)+'\n')
print('DONE',len(results),flush=True)
