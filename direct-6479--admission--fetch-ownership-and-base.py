from pathlib import Path
import json,subprocess
A=Path(__file__).parent
G='/Users/jesght/Documents/ChatGPT/sparq/upstream/.git'
MAIN='202879e9b4104f4b5ae8d9e8e5206eb2ff454b7f'
def read(name,path):
 p=subprocess.run(['/opt/homebrew/bin/gh','api',path],capture_output=True,text=True)
 (A/(name+'.stderr')).write_text(p.stderr)
 if p.returncode:raise RuntimeError('Read failed; no retry: '+p.stderr[:500])
 d=json.loads(p.stdout);(A/(name+'.json')).write_text(json.dumps(d,indent=2)+'\n');return d
b=read('ownership-budget','rate_limit')['resources'];assert b['core']['remaining']>100
for n in [5983,6435,4300,2439]:
 x=read(f'ownership-pr{n}-files',f'repos/sparq-org/sparq/pulls/{n}/files?per_page=100')
 assert len(x)<100
 print('FILES',n,len(x))
x=read('fresh-main','repos/sparq-org/sparq/git/ref/heads/main');assert x['object']['sha']==MAIN
p=subprocess.run(['git','--git-dir='+G,'fetch','--no-tags','origin',MAIN],capture_output=True,text=True)
(A/'main-fetch.stdout').write_text(p.stdout);(A/'main-fetch.stderr').write_text(p.stderr)
assert p.returncode==0,'Fetch failed; no retry'
head=subprocess.check_output(['git','--git-dir='+G,'rev-parse',MAIN+'^{commit}'],text=True).strip();assert head==MAIN
print('Fetchedexactmain',MAIN)
