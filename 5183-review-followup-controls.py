# [GPT-6 ASTRA] Bounded exact-module controls; no production mutation or outcome retry.
from pathlib import Path
import subprocess,json,hashlib,difflib
P=Path(__file__).resolve().parent;W=P.parents[1]/'worktrees/issue5183';s=(W/'crates/sparq-bench/src/update_fuzz.rs').read_text();argv=json.loads((P/'test-rustc-argv.json').read_text());records=[]
def save():(P/'control-results.json').write_text(json.dumps(records,indent=2)+'\n')
def run(name,cmd,limit=90):return subprocess.run(['python3',str(P/'run.py'),name,str(limit),*cmd]).returncode
def build(name,source,test,expected_test=101,ambient=False,expected_build=0,tests_equal=True):
 d=P/'controls'/name;d.mkdir(parents=True);(d/'update_fuzz.rs').write_text(source);(d/'main.rs').write_text('#[path="'+str(d/'update_fuzz.rs')+'"]\nmod update_fuzz;\nfn main() { update_fuzz::run(4141222487, 1); }\n');(d/'delta.diff').write_text(''.join(difflib.unified_diff(s.splitlines(True),source.splitlines(True),fromfile='candidate/update_fuzz.rs',tofile=name+'/update_fuzz.rs')))
 if tests_equal:assert source.split('#[cfg(test)]',1)[1]==s.split('#[cfg(test)]',1)[1]
 args=argv.copy();args[args.index('src/main.rs')]=str(d/'main.rs');args[args.index('--out-dir')+1]=str(d)
 rec={'name':name,'source_sha256':hashlib.sha256(source.encode()).hexdigest(),'test':test,'tests_equal':tests_equal,'build_argv':args};records.append(rec);save();rc=run(name+'-build',args);rec['build_exit']=rc;save()
 if expected_build:
  assert rc!=0
  err=(P/(name+'-build.stderr')).read_text();assert 'let chains are only allowed in Rust 2024 or later' in err
  rec['confirmed_edition_error']=True;save();return
 assert rc==0,(name,rc)
 crate=args[args.index('--crate-name')+1];extra=next(x.split('=',1)[1] for x in args if x.startswith('extra-filename='));binary=d/(crate+extra);assert binary.is_file();rec['binary_sha256']=hashlib.sha256(binary.read_bytes()).hexdigest()
 cmd=[str(binary),'--exact',test,'--test-threads=1','--nocapture']
 if ambient:cmd=['/usr/bin/env','SPARQ_FUZZ_DIVERGENCES='+str(P/'strict-allowlist.json'),*cmd]
 rec['test_exit']=run(name+'-test',cmd,30);rec['expected_test_exit']=expected_test;save();assert rec['test_exit']==expected_test,(name,rec['test_exit']);print(name,'calibrated',flush=True)
# Positive environmental calibration runs exact candidate bytes first.
assert run('marker-strict-ambient',['/usr/bin/env','SPARQ_FUZZ_DIVERGENCES='+str(P/'strict-allowlist.json'),str(P/'candidate-tests-binary'),'--exact','update_fuzz::tests::injected_marker_fails_during_actual_lexical_adjudication','--test-threads=1','--nocapture'],30)==0
old='4 => format!("{}", gen_canonical_integer(rng)),';assert s.count(old)==1
build('partial-canonical-pool-overlap',s.replace(old,'4 => format!("{}", rng.below(40)),'),'update_fuzz::tests::generated_integer_domain_is_injective_including_load')
old='''                        na.blank_nodes,
                        nb.blank_nodes,
                        one_sided(label_a, &ca, label_b, &cb)''';assert s.count(old)==1
build('omit-blank-node-dataset-details',s.replace(old,old.replace('one_sided(label_a, &ca, label_b, &cb)','String::new()')),'update_fuzz::tests::lexical_adjudication_preserves_rows_and_blank_node_structure')
old='''        let allow = UpdateDivergenceAllowlist {
            integer_lexical: true,
            state: "hermetic marker-adjudication test".into(),
        };''';assert s.count(old)==1
build('restore-ambient-marker-allowlist',s.replace(old,'        let allow = UpdateDivergenceAllowlist::load();'),'update_fuzz::tests::injected_marker_fails_during_actual_lexical_adjudication',ambient=True,tests_equal=False)
# This is an edition compatibility control, not a behavioral mutant or test pass.
build('old-source-edition2021',(P/'before.rs.txt').read_text(),None,expected_build=1,tests_equal=False)
# Inject one canonicalizer error on the third comparable() call: after both raw
# snapshots passed, during the first normalized comparable(). No engine failure is claimed.
old='fn comparable(lines: &[String], relabel: bool, what: &str) -> Result<Vec<String>, String> {';assert s.count(old)==1
fault=s.replace(old,old+'''
    #[cfg(test)] {
        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        if CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 2 {
            return Err("injected normalized comparable error".into());
        }
    }
''')
fault+='''
#[cfg(test)]
mod diagnostic_fault_probe {
    use super::*;
    #[test]
    fn normalized_error_preserves_original_details() {
        let a = vec![format!("_:a <http://ex/p> \\\"020\\\"^^{} .", XSD_INTEGER)];
        let b = vec![format!("_:b <http://ex/p> \\\"20\\\"^^{} .", XSD_INTEGER)];
        match compare("original", &a, "reference", &b, true) {
            Verdict::Differs(detail) => {
                assert!(detail.contains("injected normalized comparable error"), "{detail}");
                assert!(detail.contains("only in original:"), "{detail}");
                assert!(detail.contains("only in reference:"), "{detail}");
                assert!(detail.contains("020"), "{detail}");
            }
            _ => panic!("injected normalized comparable error must fail"),
        }
    }
}
'''
# Unescape just the diagnostic fixture's Rust string literal quotes.
fault=fault.replace('\\\\\\"','\\"')
probe='update_fuzz::diagnostic_fault_probe::normalized_error_preserves_original_details'
build('normalized-error-fault-positive',fault,probe,expected_test=0,tests_equal=False)
old='''                    (Err(e), _) | (_, Err(e)) => {
                        return Verdict::Differs(format!(
                            "integer-lexical adjudication refused: {e}\\n{}",
                            one_sided(label_a, &ca, label_b, &cb)
                        ));
                    }''';assert fault.count(old)==1
build('normalized-error-old-detail-control',fault.replace(old,'                    (Err(e), _) | (_, Err(e)) => return Verdict::Differs(e),'),probe,tests_equal=False)
print('ALL CONTROLS TERMINAL',flush=True)
