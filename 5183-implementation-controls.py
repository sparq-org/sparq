# [GPT-6 ASTRA] Compile exact-module mutants against the already verified dependency set.
from pathlib import Path
import json,subprocess,hashlib,difflib
P=Path(__file__).resolve().parent;R=P.parents[1];W=R/'worktrees/issue5183';source=(W/'crates/sparq-bench/src/update_fuzz.rs').read_text();base=json.loads((P/'test-rustc-argv.json').read_text());records=[]
controls=[
 ('pool-overlap','let v = CANONICAL_INTEGER_VALUES + 3 * offset + style;','let v = offset;','generated_integer_domain_is_injective_including_load'),
 ('load-overlap','gen_canonical_integer(rng),\n            XSD_INTEGER','(gen_canonical_integer(rng) + 20),\n            XSD_INTEGER','generated_integer_domain_is_injective_including_load'),
 ('omit-term-injectivity','if previous != l.value() {','if false && previous != l.value() {','lexical_collisions_across_contexts_and_nested_terms_fail'),
 ('omit-nested-injectivity','record_lexical_terms(&t.object, integers, blank_nodes)?;','let _ = (&t.object, integers, blank_nodes);','lexical_collisions_across_contexts_and_nested_terms_fail'),
 ('omit-row-guard','if out.windows(2).any(|rows| rows[0] == rows[1]) {','if false && out.windows(2).any(|rows| rows[0] == rows[1]) {','lexical_adjudication_preserves_rows_and_blank_node_structure'),
 ('omit-normalizer-recursion','oxigraph_normalized_term(&inner.object),','inner.object.clone(),','nested_noncanonical_terms_remain_exact_in_sparq'),
 ('blind-adjudication','if allow_integer_lexical {','if allow_integer_lexical {\n        return Verdict::AdjudicatedIntegerLexical;','injected_marker_fails_during_actual_lexical_adjudication'),
]
for name,old,new,test in controls:
 assert source.count(old)==1,(name,source.count(old),old)
 d=P/'controls'/name;d.mkdir(parents=True);mutated=source.replace(old,new);(d/'update_fuzz.rs').write_text(mutated);(d/'delta.diff').write_text(''.join(difflib.unified_diff(source.splitlines(True),mutated.splitlines(True),fromfile='candidate/update_fuzz.rs',tofile='mutant/update_fuzz.rs')))
 # No test is changed: all mutations precede the test module.
 assert source.split('#[cfg(test)]',1)[1]==mutated.split('#[cfg(test)]',1)[1]
 wrapper=d/'main.rs';wrapper.write_text('#[path="'+str(d/'update_fuzz.rs')+'"]\nmod update_fuzz;\nfn main() { update_fuzz::run(4141222487, 1); }\n')
 argv=base.copy();argv[argv.index('src/main.rs')]=str(wrapper);argv[argv.index('--out-dir')+1]=str(d)
 rec={'name':name,'test':test,'test_suffix_unchanged':True,'source_sha256':hashlib.sha256(mutated.encode()).hexdigest(),'build_argv':argv}
 records.append(rec);(P/'control-results.json').write_text(json.dumps(records,indent=2)+'\n')
 c=subprocess.run(['python3',str(P/'run.py'),name+'-build','600',*argv]);rec['build_exit']=c.returncode
 if c.returncode:break
 binary=d/'issue5183_injective_tests-dd029590d0327647';assert binary.is_file();rec['binary_sha256']=hashlib.sha256(binary.read_bytes()).hexdigest()
 c=subprocess.run(['python3',str(P/'run.py'),name+'-test','150',str(binary),'--exact','update_fuzz::tests::'+test,'--nocapture','--test-threads=1']);rec['test_exit']=c.returncode;rec['killed']=c.returncode==101
 (P/'control-results.json').write_text(json.dumps(records,indent=2)+'\n');print(json.dumps(rec|{'build_argv':'see saved record'}),flush=True)
 if not rec['killed']:break
print('CONTROLS FINISHED',len(records),flush=True)
