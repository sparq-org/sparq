# [GPT-6 Astra] Bounded offline isolated replay; fails closed on disk/time limits.
from pathlib import Path
import json,os,shutil,subprocess,time,signal
E=Path(__file__).resolve().parent; H=E/'harness'; target=E/'target'
tool=Path('/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin')
env=dict(os.environ,PATH=str(tool)+os.pathsep+os.environ['PATH'],RUSTC=str(tool/'rustc'),RUSTDOC=str(tool/'rustdoc'),CARGO_TARGET_DIR=str(target),CARGO_BUILD_JOBS='2',CARGO_NET_OFFLINE='true',CARGO_INCREMENTAL='0')
records=[]
start=time.monotonic(); floor=6*1024**3
for name,argv,expected in [('toolchain',[str(tool/'rustc'),'-Vv'],0),('format',[str(tool/'rustfmt'),'--edition','2024',str(H/'src/main.rs')],0),('resolve',[str(tool/'cargo'),'metadata','--offline','--format-version','1'],0),('build',[str(tool/'cargo'),'build','--locked','--offline'],0),('replay',[str(target/'debug/canon-relabel-replay')],1)]:
    before=shutil.disk_usage(E).free
    assert before>=floor,('free_disk_floor',before)
    stdout=E/(name+('.json' if name=='resolve' else '.log')); stderr=E/(name+'.stderr.log')
    with stdout.open('wb') as out,stderr.open('wb') as err:
        p=subprocess.Popen(argv,cwd=H,env=env,stdout=out,stderr=err,start_new_session=True)
        why=None
        while p.poll() is None:
            if time.monotonic()-start>300:why='300-second total cap'
            if shutil.disk_usage(E).free<floor:why='6GiB free disk floor'
            if why:
                os.killpg(p.pid,signal.SIGTERM);p.wait(timeout=10);break
            time.sleep(0.25)
    record=dict(name=name,argv=argv,cwd=str(H),exit=p.returncode,expected_exit=expected,free_before=before,free_after=shutil.disk_usage(E).free,elapsed_total_seconds=time.monotonic()-start,limit=why)
    records.append(record);(E/'commands.json').write_text(json.dumps(records,indent=2)+'\n');print(record,flush=True)
    if why or p.returncode!=expected:break
(E/'environment.json').write_text(json.dumps({k:env[k] for k in ('RUSTC','RUSTDOC','CARGO_TARGET_DIR','CARGO_BUILD_JOBS','CARGO_NET_OFFLINE','CARGO_INCREMENTAL')},indent=2)+'\n')
