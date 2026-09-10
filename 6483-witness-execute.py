from pathlib import Path
import json,os,subprocess,time,signal,hashlib,difflib,sys
p=Path(__file__).resolve().parent;r=p.parent.parent;w=r/'worktrees/issue6485';old=r/'direct-5183/review-followup';sha=lambda b:hashlib.sha256(b).hexdigest();initial=json.loads((p/'initial.json').read_text());start=time.monotonic();receipts=[];resources=[]
def put(n,v):(p/n).write_text(json.dumps(v,indent=2)+'\n' if not isinstance(v,str) else v)
def check():
 free=os.statvfs(p).f_bavail*os.statvfs(p).f_frsize;size=sum(q.stat().st_blocks*512 for q in p.rglob('*') if q.is_file() and not q.is_symlink());row={'elapsed_seconds':time.monotonic()-start,'free_bytes':free,'new_allocated_bytes':size};resources.append(row);put('resources.json',resources);assert free>=2147483648,'free floor';assert size<62914560,'64MiB cap with4MiB export reserve';assert row['elapsed_seconds']<90,'90-second cap'
source=(p/'production-source.rs.txt').read_text();suffix='''
// [GPT-6 Astra] Isolated Copilot witness; observes the strict identical-lexical path.
#[cfg(test)]
mod copilot_duplicate_probe {
    use super::*;

    fn observation(case: &str, allow: bool, verdict: &Verdict) {
        let (name, detail) = match verdict {
            Verdict::Same => ("Same", None),
            Verdict::AdjudicatedIntegerLexical => ("AdjudicatedIntegerLexical", None),
            Verdict::Differs(detail) => ("Differs", Some(detail.as_str())),
        };
        println!("duplicate_witness {}", serde_json::json!({
            "case": case, "allow_integer_lexical": allow, "verdict": name, "detail": detail
        }));
    }

    #[test]
    fn distinct_control_and_identical_lexical_observation() {
        let p = format!("_:x <http://ex/p> \\\"020\\\"^^{XSD_INTEGER} .");
        let q = format!("_:x <http://ex/q> \\\"021\\\"^^{XSD_INTEGER} .");
        let left = vec![p.clone(), p.clone(), q.clone()];
        let distinct = vec![p.replace("020", "20"), q.replace("021", "21"), q.replace("021", "21")];
        let identical = vec![p, q.clone(), q];
        println!("duplicate_inputs {}", serde_json::json!({"left": left, "distinct": distinct, "identical": identical}));
        for allow in [false, true] {
            let control = compare("left", &left, "right", &distinct, allow);
            observation("distinct_lexical_control", allow, &control);
            assert!(matches!(control, Verdict::Differs(_)), "current differing-lexical control must refuse");
            let observed = compare("left", &left, "right", &identical, allow);
            observation("identical_lexical_redistribution", allow, &observed);
        }
    }
}
'''
# Literal Rust source requires escaped quotes, not an extra backslash before them.
suffix=suffix.replace('\\\\\\"','\\"')
check();assert sha(Path(json.loads((p/'old-compiler-argv.json').read_text())[0]).read_bytes())==initial['compiler_sha256'];put('test-suffix.rs.txt',suffix);put('update_fuzz.rs',source+suffix);put('source.diff',''.join(difflib.unified_diff(source.splitlines(True),(source+suffix).splitlines(True),fromfile='production/update_fuzz.rs',tofile='witness/update_fuzz.rs')))
q=p/'harness/crates/sparq-bench';driver='// [GPT-6 Astra] Actual module test driver; original public entry remains compiled.\n#[path = "'+str(p/'update_fuzz.rs')+'"]\nmod update_fuzz;\nfn main() { update_fuzz::run(4141222487, 1); }\n';put('driver.rs',driver)
argv=json.loads((p/'old-compiler-argv.json').read_text());argv[argv.index('--crate-name')+1]='issue6483_duplicate_witness';argv[argv.index('--out-dir')+1]=str(p/'out')
for i,x in enumerate(argv):
 if x.endswith('/src/main.rs') or x=='src/main.rs':argv[i]=str(p/'driver.rs')
 if x.startswith('metadata='):argv[i]='metadata=issue6483_duplicate_fixed'
 if x.startswith('extra-filename='):argv[i]='extra-filename=-fixed'
assert '--edition=2021' in argv and '--test' in argv
put('compiler-argv.json',argv);env={k:v for k,v in os.environ.items() if not k.startswith(('SPARQ_','CARGO_PROFILE_')) and k not in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS')};controlled=json.loads((old/'commands.json').read_text())[1]['environment'];controlled.update(TMPDIR=str(p/'tmp'),CARGO_MANIFEST_DIR=str(q));env.update(controlled)
put('protocol.json',{'source_head':initial['head'],'source_unchanged_prefix_sha256':initial['source_sha256'],'method':'One direct Rust2021 driver/module build and one filtered test. Current distinct lexical fixture assertsDiffers; identical lexical variant prints actualverdict without expectation, allowfalse/true in same test.','limits':{'seconds':90,'new_bytes':67108864,'free_bytes':2147483648,'jobs':1},'controlled_environment':controlled,'initial_free':initial['free_bytes'],'no_dependency_build_or_retry':True})
def run(name,cmd,limit):
 check()
 for d in initial['externs']:assert sha(Path(d['path']).read_bytes())==d['sha256']
 t=time.monotonic();stopped=None
 with (p/(name+'.stdout')).open('wb') as out,(p/(name+'.stderr')).open('wb') as err:
  proc=subprocess.Popen(cmd,cwd=q,env=env,stdout=out,stderr=err,start_new_session=True)
  while proc.poll() is None:
   try:check();assert time.monotonic()-t<limit,'per-command cap'
   except Exception as e:
    stopped=str(e);os.killpg(proc.pid,signal.SIGTERM)
    try:proc.wait(timeout=3)
    except subprocess.TimeoutExpired:os.killpg(proc.pid,signal.SIGKILL);proc.wait()
    break
   time.sleep(.35)
  code=proc.wait()
 row={'name':name,'argv':cmd,'exit':code,'seconds':time.monotonic()-t,'stopped':stopped};receipts.append(row);put('commands.json',receipts);check();print(json.dumps(row),flush=True);assert stopped is None and code==0,'Stop: command failure'
try:
 run('build',argv,60);binary=p/'out/issue6483_duplicate_witness-fixed';assert binary.is_file();put('binary.json',{'path':str(binary),'sha256':sha(binary.read_bytes()),'bytes':binary.stat().st_size,'source_sha256':sha((p/'update_fuzz.rs').read_bytes()),'driver_sha256':sha((p/'driver.rs').read_bytes())})
 run('test',[str(binary),'--exact','update_fuzz::copilot_duplicate_probe::distinct_control_and_identical_lexical_observation','--test-threads=1','--nocapture'],20)
 put('phase-result.json',{'status':'complete','seconds':time.monotonic()-start,'commands_terminal':2});print('BOTH COMMANDS TERMINAL',flush=True)
except Exception as e:
 put('phase-result.json',{'status':'stopped','reason':str(e),'seconds':time.monotonic()-start,'commands_terminal':len(receipts)});print('STOP',str(e),flush=True);sys.exit(1)
