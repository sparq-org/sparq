# GPT-6 Astra: source compilation/lints with exact previously executed compiler dependency args.
from pathlib import Path
import json,shlex,subprocess
v=Path(__file__).resolve().parent;marker='/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc --crate-name sparq_engine '
def engine_args(log):
 lines=[x for x in (v/log).read_text().splitlines() if marker in x];assert len(lines)==1
 return shlex.split(lines[0][lines[0].index(marker):].removesuffix('`'))
def run(label,args):
 r=subprocess.run(['python3',str(v/'run.py'),label,*args],stdout=subprocess.PIPE,stderr=subprocess.STDOUT);print(r.stdout.decode());return json.loads((v/(label+'.json')).read_text())
def reshape(args,label,serial=False,lint=False):
 out=[];i=0;folder=v/label;folder.mkdir(exist_ok=True)
 while i<len(args):
  x=args[i]
  if x=='--cfg' and serial:i+=2;continue
  if x=='--out-dir':out.extend([x,str(folder)]);i+=2;continue
  if x=='--emit=dep-info,metadata,link' and lint:out.append('--emit=metadata');i+=1;continue
  if x=='-C' and args[i+1].startswith('metadata='):out.extend([x,'metadata='+label]);i+=2;continue
  if x=='-C' and args[i+1].startswith('extra-filename='):out.extend([x,'extra-filename=-'+label]);i+=2;continue
  out.append(x);i+=1
 if lint:out[0]=out[0].replace('/rustc','/clippy-driver');out+=['-D','warnings']
 return out
r=run('direct-serial-compile',reshape(engine_args('build-default.log'),'direct-serial',serial=True));assert r['exit_code']==0,r
r=run('direct-clippy-lib',reshape(engine_args('build-features.log'),'direct-clippy',lint=True));assert r['exit_code']==0,r
args=json.loads((v/'compile-module.json').read_text())['command'];args[0]=args[0].replace('/rustc','/clippy-driver');args=[str(v/'clippy-tests.rmeta') if x==str(v/'binary/actual-budget') else x for x in args];args+=['--emit=metadata','-D','warnings'];r=run('direct-clippy-tests',args);assert r['exit_code']==0,r
