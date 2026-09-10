# [GPT-6 ASTRA] Export fixed observations and provenance; no execution of cases here.
from pathlib import Path
import json,hashlib,shlex,shutil,subprocess,os
out=Path(__file__).resolve().parent;r=out.parents[1];wt=r/'worktrees/issue5183';target=r/'direct-5983/implementation/target'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def save(p,obj):(out/p).write_text(json.dumps(obj,indent=2)+'\n')
def allocated(path):
 total=0
 for d,_,files in os.walk(path):
  for n in files:
   try:total+=(Path(d)/n).stat().st_blocks*512
   except FileNotFoundError:pass
 return total
phase=json.loads((out/'phase-result.json').read_text());assert phase['status']=='complete'
observations=json.loads((out/'observations.json').read_text());engine=json.loads((r/'direct-5183/engine-witness/observations.json').read_text());resources=json.loads((out/'resources.json').read_text());commands=json.loads((out/'commands.json').read_text())
rows=[]
for obs in observations:
 case=obs['case'];row={'case':case,'oxigraph':{'before_quads':len(obs['before']),'where_rows':len(obs['where_rows']),'distinct_blank_nodes':obs['distinct_blank_nodes'],'after_quads':len(obs['after']),'lexical_terms':sorted({x[1] for x in obs['where_rows']})},'sparq':{}}
 for variant,items in engine.items():
  for v in items:
   if v['case']==case:row['sparq'][variant+'/'+v['path']]={'before_quads':len(v['before']),'where_rows':len(v['where_rows']),'distinct_blank_nodes':len(set(v['blank_nodes'])),'after_quads':len(v['after']),'lexical_terms':v['lexical_terms']}
 rows.append(row)
save('comparison.json',rows)
log=(out/'build.stderr').read_text();compiler=[];deps={}
for line in log.splitlines():
 if 'Running `' not in line or '/bin/rustc ' not in line:continue
 start=line.index('/Users/jesght/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin/rustc ')
 argv=shlex.split(line[start:].rstrip('`'))
 if '--crate-name' not in argv:continue
 name=argv[argv.index('--crate-name')+1]
 if name not in ['issue5183_oxigraph_witness','oxigraph','spargebra','spareval','oxrdf']:continue
 compiler.append({'crate':name,'argv':argv})
 for i,arg in enumerate(argv):
  if arg=='--extern':
   dep,path=argv[i+1].split('=',1);p=Path(path);deps[str(p)]={'name':dep,'path':str(p),'bytes':p.stat().st_size,'sha256':sha(p)}
assert any(x['crate']=='oxigraph' for x in compiler) and any(x['crate']=='issue5183_oxigraph_witness' for x in compiler)
save('compiler-provenance.json',{'selected_actual_rustc_invocations':compiler,'referenced_artifacts':list(deps.values()),'profile':'opt-level3/codegen-units16/lto=false/panic=unwind/incremental=false. Cargo omits explicit rustc options where defaults implement the requested setting; original environment and actual argv retained.','full_raw_log':'build.stderr'})
shutil.copyfile(target/'release/issue5183-oxigraph-witness.d',out/'issue5183-oxigraph-witness.d')
shutil.copyfile(r/'direct-5183/engine-witness/binary-source-provenance.json',out/'earlier-engine-binary-source-provenance.json')
shutil.copyfile(r/'direct-5183/admission/normative-semantics.json',out/'normative-semantics.json')
check={'head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=wt,text=True).strip(),'branch':subprocess.check_output(['git','branch','--show-current'],cwd=wt,text=True).strip(),'status':subprocess.check_output(['git','status','--porcelain'],cwd=wt,text=True),'repository_lock_sha256':sha(wt/'Cargo.lock'),'matches_before_lock':sha(wt/'Cargo.lock')==sha(out/'repository-Cargo.lock')}
assert check['status']=='' and check['matches_before_lock'];save('final-source-state.json',check)
save('auxiliary-check-limitations.json',{'process_inventory':{'command':'ps -axo pid,comm (filtered for cargo/rustc)','exit':1,'stderr':'zsh:1: operation not permitted: ps','disposition':'No retry/alternate process inventory. Own runner enforced serial cargo/jobs1; it completed and no child remains.'},'optional_cached_file_integrity_check':{'status':'Not established','error':'FileNotFoundError: cached aho-corasick-1.1.4/.cargo-checksum.json absent','disposition':'Stopped optional whole-cache file verification; no fetch/retry. Cargo offline pin equality, selected actual source/manifest hashes and linked artifact hashes preserved; no all-cache source authentication claim.'},'first_export_parser':{'status':'Corrected export-only parse','error':'ValueError --crate-name absent for a build-script command containing RUSTC environment text','disposition':'Ignore non-rustc lines lacking --crate-name. No rebuild or query rerun.'}})
report={'author':'GPT-6 Astra xhigh','status':'COMPLETE_FIXED_REFERENCE_WITNESS','production_changes':False,'conclusion':'Oxigraph0.5.9 collapses the two same-value integer lexical terms before WHERE evaluation, changing solution cardinality and fresh template blank-node count. Sparq parent/main public witnesses retain both RDF terms. This establishes a reference-cardinality explanation for the reduction independently of canonicalization.','observed_cases':rows,'source_cause':{'source':'oxigraph0.5.9/src/storage/numeric_encoder.rs','integer_dispatch_lines':'583-595','parse_integer_line':870,'decode_integer_line':1140,'mechanism':'Valid integer lexical text is parsed into EncodedTerm::IntegerLiteral(value); decoding creates Literal::from(value). Raw Store iteration and SELECT demonstrate one canonical term for 8 plus 008.'},'scope':{'operations':2,'cases':3,'reference_runs':1,'retries':0,'reference_api':['Store::new','Store::update','Store::query','Store::iter'],'reference_features':['rdf-12'],'disabled':['default/rocksdb','http-client'],'resolved_packages':62,'production_lock_dependency_drift':[],'patched_parser':'vendored spargebra0.4.6 at repository head781f667c','canonicalizer_used':False,'controls':'8-only yields1/1/1; 8+9 yields2/2/2, matching both earlier Sparq revisions and both update paths.'},'normative_interpretation':'RDF literal term identity includes lexical form; same integer value does not make these two RDF terms identical. INSERT blank nodes are fresh per WHERE solution. Sparq preserves two terms/solutions; the reference stores a smaller graph. This does not yet establish the cause of all eight CI mismatches.','limits':['NOT the full ten-operation seed4141222487 CI replay; seven other seeds unexecuted.','Earlier Sparq witnesses used qualified cached parent/main-equivalent libraries with default engine/core features, not CI feature parity; exact attribution copied unchanged.','Rust1.97.1/aarch64 macOS/O3/unwind/no-LTO/codegen16 is not Linux CI release-fast thin-LTO/abort.','Blank-node labels are incidental; compare counts within results, not identifiers across executions.','No comparator, canonicalizer, generator, production input, engine source, dependency version or gate changed.','Dependency warnings retained; not a full crate/workspace lint/conformance run.','No unsupported whole-cache source authentication claim; actual selected source, manifest and artifact hashes retained.'],'resources':{'elapsed_compile_run_seconds':phase['elapsed_seconds'],'maximum_sampled_aggregate_growth_bytes':max(x['aggregate_new_bytes'] for x in resources),'minimum_sampled_free_bytes':min(x['free_bytes'] for x in resources),'sample_interval_seconds':2,'limits':{'seconds':600,'growth_bytes':512*1024**2,'free_bytes':2*1024**3},'meaning':'Allocated disk bytes; no performance, allocator heap or RSS measurement claim.'},'next_small_step':'Separately authorize unchanged update_fuzz exact seed4141222487 on qualified parent/main sources with CI-relevant features, strict rebuild/in-place check, generator/LOAD sandbox, allowlist and comparator preserved. Capture failure step and raw pre-step graphs. Do not normalize production inputs or weaken comparisons based only on this reduction.','no_commands_pending':True}
save('report.json',report)
packet=['# Issue5183 pinned reference witness','Authored by GPT-6 Astra xhigh. Public synthetic data only; no production patch.','## Observations and limits',json.dumps({k:report[k] for k in ['conclusion','scope','normative_interpretation','limits','next_small_step']},indent=2),'## Comparison',json.dumps(rows,indent=2),'## Exact raw reference records',(out/'fixed-cases.stdout').read_text(),'## Actual harness','```rust',(out/'harness/src/main.rs').read_text(),'```','## Pinned source','oxigraph0.5.9, default-features=false, rdf-12; repository-pinned dependencies; vendored spargebra0.4.6. No RocksDB/HTTP.']
lines=(out/'source/oxigraph/src/storage/numeric_encoder.rs').read_text().splitlines()
for lo,hi in [(583,595),(870,872),(1137,1142)]:packet+=['```rust','\n'.join(f'{n}: {lines[n-1]}' for n in range(lo,hi+1)),'```']
packet+=['## Primary references','RDF1.1 Concepts §3.3 https://www.w3.org/TR/rdf11-concepts/#section-Graph-Literal','SPARQL1.1 Update §3.1.3/4.2.3 https://www.w3.org/TR/sparql11-update/#deleteInsert','## Build identity',json.dumps({'compiler':(out/'rustc-version.txt').read_text(),'harness_sha256':sha(out/'harness/src/main.rs'),'binary':json.loads((out/'binary.json').read_text()),'resolved_lock_sha256':sha(out/'harness/Cargo.lock'),'command_outcomes':[{k:c[k] for k in ['name','exit','elapsed_seconds']} for c in commands],'resources':report['resources']},indent=2),'Omitted host paths, raw environment/build logs and full dependency sources from this public packet. Named diagnostics remain in the local bundle. No full-source or full-workspace review claim.']
(out/'review-packet.md').write_text('\n\n'.join(packet)+'\n')
current={'free_bytes':shutil.disk_usage(out).free,'target_allocated_bytes':allocated(target),'output_allocated_bytes':allocated(out)};current['total_new_allocated_bytes']=max(0,current['target_allocated_bytes']-phase['initial_target_allocated_bytes'])+current['output_allocated_bytes'];assert current['free_bytes']>=2*1024**3 and current['total_new_allocated_bytes']<512*1024**2;save('final-resource.json',current)
print(json.dumps({'packet_bytes':(out/'review-packet.md').stat().st_size,'resource':current,'observations':[{'case':v['case'],'where_rows':len(v['where_rows']),'blank_nodes':v['distinct_blank_nodes']} for v in observations]}))
