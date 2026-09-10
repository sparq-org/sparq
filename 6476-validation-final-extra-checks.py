from pathlib import Path
import json,shlex,subprocess
v=Path(__file__).resolve().parent;tool='/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc';deps=Path('/private/tmp/sparq-pr6049/.throughput-monitor/direct-5983/implementation/target/release/deps')
def run(label,args):
 r=subprocess.run(['python3',str(v/'run.py'),label,*args],stdout=subprocess.PIPE,stderr=subprocess.STDOUT);print(r.stdout.decode());j=json.loads((v/(label+'.json')).read_text());assert j['exit_code']==0 and j['stop'] is None,j
# Real no-parallel engine variant uses unchanged pinned dependency artifacts; core remains parallel-enabled.
base=[tool,'--edition','2021','-C','opt-level=3','-C','panic=unwind','-C','debuginfo=0','-L','dependency='+str(deps),'--extern','sparq_core='+str(deps/'libsparq_core-127354fe878a2f23.rlib'),'--extern','oxrdf='+str(deps/'liboxrdf-4ecd756ddd7ab1ec.rlib')]
run('compile-public-serial',base+[str(v/'public/src/main.rs'),'--extern','sparq_engine='+str(v/'direct-serial/libsparq_engine-direct-serial.rlib'),'-o',str(v/'binary/public-serial')]);run('public-serial',[str(v/'binary/public-serial'),'--require-restoration'])
# Compile the task-private source copy with only the recorded observer delta.
marker=tool+' --crate-name sparq_engine ';line=next(x for x in (v/'build-default.log').read_text().splitlines() if marker in x);args=shlex.split(line[line.index(marker):].removesuffix('`'));out=v/'parallel-probe/lib';out.mkdir(exist_ok=True)
for i,x in enumerate(args):
 if x.endswith('/crates/sparq-engine/src/lib.rs'):args[i]=str(v/'parallel-probe/src/lib.rs')
 elif x=='--out-dir':args[i+1]=str(out)
 elif x=='metadata=40d833ef5410a7ad':args[i]='metadata=parallelprobe6476'
 elif x.startswith('extra-filename='):args[i]='extra-filename=-parallelprobe6476'
run('compile-parallel-probe',args)
run('compile-parallel-public',base+[str(v/'parallel-public.rs'),'--extern','rayon='+str(deps/'librayon-fb353b242d14de3d.rlib'),'--extern','sparq_engine='+str(out/'libsparq_engine-parallelprobe6476.rlib'),'-o',str(v/'binary/parallel-public')])
# A watchdog bounds an observer rendezvous failure; no stress or timing assertions.
watch=v/'bounded-parallel-run.py';watch.write_text('import subprocess,sys\nr=subprocess.run([sys.argv[1]],timeout=30)\nsys.exit(r.returncode)\n')
run('parallel-public',['python3',str(watch),str(v/'binary/parallel-public')])
log=(v/'parallel-public.log').read_text();threads=set(__import__('re').findall(r'BUDGET_PROBE path=scan_json thread=(ThreadId\(\d+\))',log));assert len(threads)==2,threads;(v/'parallel-observation.json').write_text(json.dumps({'path':'actual scan JSON par_chunks snapshot hit','threads':sorted(threads),'thread_count':len(threads),'rows':60000,'instrumentation_only':True,'production_delta':False},indent=2)+'\n')
