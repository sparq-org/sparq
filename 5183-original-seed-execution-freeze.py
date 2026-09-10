from pathlib import Path
import hashlib,json,re,shlex,shutil,subprocess,time,os
R=Path('/private/tmp/sparq-pr6049/.throughput-monitor'); P=R/'direct-5183/original-seed-execution'; W=R/'worktrees/issue5183'; T=R/'direct-5983/implementation/target'
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def save(n,o): (P/n).write_text(json.dumps(o,indent=2)+'\n')
def copy(src,dest):
 q=P/dest;q.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(src,q)
def git(*args):return subprocess.check_output(['git',*args],cwd=W,text=True).strip()
assert git('rev-parse','HEAD')=='781f667c19a8ebb779cfccb24b05ea432360b025'
assert not git('status','--porcelain')
commands=json.loads((P/'commands.json').read_text()); phase=json.loads((P/'phase-result.json').read_text()); resources=json.loads((P/'resources.json').read_text())
assert phase['status']=='complete' and all(c.get('exit') is not None for c in commands)
ci=R/'direct-5183/readiness/first-case.txt'; sequence=(R/'direct-5183/readiness/captured-sequence.txt').read_text().strip()
ci_clean='\n'.join(re.sub(r'^\d{4}-\d\d-\d\dT\S+Z ?', '',l) for l in ci.read_text().splitlines())
def failure(s):return s.split('FIRST FAILING CASE:\n',1)[1].split('\nrepro:',1)[0].strip()
comp={'ci_run':34458877088,'ci_job':102811674711,'seed':4141222487,'variants':{}}
source_verified={}; compiler={}
selected=['Cargo.toml','Cargo.lock','rust-toolchain.toml','.github/workflows/differential-update.yml','crates/sparq-bench/Cargo.toml','crates/sparq-bench/src/update_fuzz.rs','bench/differential-divergences.json','crates/sparq-engine/Cargo.toml','crates/sparq-engine/src/update.rs','crates/sparq-core/Cargo.toml','crates/sparq-canon/Cargo.toml','crates/sparq-canon/src/lib.rs','vendor/spargebra/Cargo.toml']
for v in ['parent','main']:
 stdout=(P/f'{v}-seed.stdout').read_text(); obs=stdout.split('--- full sequence ---\n',1)[1].strip()
 comp['variants'][v]={'sequence_equals_ci':obs==sequence,'failure_equals_ci':failure(stdout)==failure(ci_clean),'generated_indices':[int(i) for i in re.findall(r'^\[(\d+)\]',obs,re.M)],'stdout_except_startup_sha256':hashlib.sha256(stdout.split('\n',1)[1].encode()).hexdigest(),'stderr':(P/f'{v}-seed.stderr').read_text(),'exit':next(c['exit'] for c in commands if c['name']==v+'-seed')}
 assert comp['variants'][v]['sequence_equals_ci'] and comp['variants'][v]['failure_equals_ci'] and comp['variants'][v]['generated_indices']==list(range(10))
 before=json.loads((P/f'{v}-source-before.json').read_text());after=json.loads((P/f'{v}-source-after.json').read_text());assert before==after
 source=P/'parent-source' if v=='parent' else W
 assert all(sha(source/x['path'])==x['sha256'] for x in after)
 setup=json.loads((P/f'{v}-setup-proof.json').read_text());h=P/v/'harness/crates/sparq-bench'
 assert sha(h/'src/main.rs')==setup['harness_sha256'] and sha(h/'Cargo.lock')==setup['lock_sha256']
 assert sha(P/v/'harness/bench/differential-divergences.json')==setup['allowlist_sha256']
 b=json.loads((P/f'{v}-binary.json').read_text());binary=P/f'issue5183-original-{v}-binary';assert sha(binary)==b['sha256']
 source_verified[v]={'source_head':setup['source_head'],'hashed_source_files':len(after),'before_after_identical':True,'all_current_hashes_match_after':True,'dependency_pin_drift':setup['dependency_pin_drift'],'harness_lock_allowlist_binary_hashes_verified':True}
 for rel in selected:
  if (source/rel).is_file():copy(source/rel,Path('source')/v/rel)
 inv=[]
 for line in (P/f'{v}-build.stderr').read_text().splitlines():
  if 'Running `' not in line or '--crate-name ' not in line:continue
  raw=line.split('Running `',1)[1].rsplit('`',1)[0];tokens=shlex.split(raw);name=tokens[tokens.index('--crate-name')+1]
  if name not in ['sparq_engine','sparq_core','sparq_canon','sparq_substrate','spargebra','oxigraph','rdf_canon','oxrdf'] and not name.startswith('issue5183_'):continue
  ri=next(i for i,t in enumerate(tokens) if t.endswith('/rustc'));argv=tokens[ri:]
  artifacts=[]
  for i,t in enumerate(argv):
   if t=='--extern':
    n,path=argv[i+1].split('=',1);fp=Path(path)
    artifacts.append({'crate':n,'path':path,'bytes':fp.stat().st_size,'sha256':sha(fp)})
  inv.append({'crate':name,'actual_rustc_argv':argv,'extern_artifacts_now_hashed':artifacts})
 assert {'sparq_core','sparq_engine','sparq_canon',f'issue5183_original_{v}'}.issubset({i['crate'] for i in inv})
 compiler[v]=inv
 dep=T/'release'/f'issue5183-original-{v}.d'
 if dep.exists():copy(dep,f'{v}-binary-depinfo.d')
copy(ci,'ci-first-case.txt');copy(R/'direct-5183/readiness/captured-sequence.txt','ci-generated-sequence.txt')
save('comparison.json',comp);save('final-source-verification.json',source_verified);save('compiler-linked-provenance.json',compiler)
maxgrowth=max(x['new_allocated'] for x in resources);minfree=min(x['free'] for x in resources)
report={
 'author':'GPT-6 Astra xhigh','status':'COMPLETED_OBSERVED_MISMATCH_BOTH_REVISIONS','production_changes':False,
 'source_revisions':{v:source_verified[v]['source_head'] for v in source_verified},'worktree_clean':True,
 'execution':{'phase_seconds':phase['seconds'],'builds':2,'seed_invocations':2,'seed':4141222487,'count_each':1,'exit_each':1,'no_outcome_retries':True,'terminal_receipts':'commands.json / phase-result.json','pending_commands':False,'completion_basis':'The saved phase is complete, every named child command has a terminal exit, and execute.py polls/waits for those children. Coordinator also found the old exec session unavailable. No global process inventory is claimed; no test was restarted during this export.'},
 'observations':[
  'Both revisions generated exactly the ten CI operations, with identical first-failure detail including the extra integer lexical 008 and c14n8 blank node.',
  'Failure is at zero-based index 8: nine generated requests were applied, then the reference dataset comparison returned early. The printed index-9 ADD request was generated but not executed.',
  'The same witness on d41, before PR6478, excludes PR6478 as the introduction of this first-seed mismatch under the qualified diagnostic configuration.',
  'Source ordering establishes that the strict rebuild-vs-in-place dataset comparison passed before the failing rebuild-vs-Oxigraph comparison, including at index 8. Later index-8 reference/probe checks and index-9 checks were not reached.',
  'The startup line confirms the committed integer-lexical adjudication was enabled. Printed counts are accumulated only for completely successful seeds, so their zero values do not imply zero work or absence of earlier adjudication.',
  'The diagnostic prints the original canonical difference after normalization fails to remove residual differences; it is not a raw post-normalization diff.'
 ],
 'interpretation':{'established':'The original first-seed failure is reproducible before and after PR6478 with unchanged generator, comparator, parser and LOAD sandbox. Existing reduced public-API results independently show Sparq retains two distinct integer lexical terms/WHERE rows/fresh blank nodes while pinned Oxigraph collapses the two terms into one before query evaluation.','supported_explanation':'At sequence index 4 the lexical pair is inserted; index 8 allocates a fresh template blank node per WHERE solution. Post-hoc lexical normalization can merge literals, but cannot undo the additional blank node already allocated from the larger solution multiset.','limits':'This explanation combines the unchanged sequence/source with prior reduced witnesses; this run did not instrument every intermediate WHERE row. Only the first of eight CI mismatches was executed. No generic comparator change or engine repair is validated.'},
 'preservation':{'module_sha256':'0807388f84792f011b868456269a36bf74e5e463c67dfb71d1be3ad9d35aecf7','allowlist_sha256':'2b6821405a540221cadde3df8ca2468cf141d1a907c44d75e881f6feadbdedd2','generator_parser_canon_load_and_strict_guards':'unchanged qualified tracked sources, source before/after proofs identical','manifest_dir':'Each uniquely named harness preserves crates/sparq-bench relative layout and the exact committed ../../bench allowlist; SPARQ_FUZZ_DIVERGENCES unset.','dependency_pins':'No registry dependency version/source/checksum drift. Metadata feature closure matches the frozen minimal readiness closure for both revisions.'},
 'features':{k:v for k,v in json.loads((P/'main-setup-proof.json').read_text())['features'].items() if k.split('@')[0] in ['sparq-engine','sparq-core','sparq-canon','sparq-substrate','spargebra','oxigraph','oxrdf','rdf-canon','oxttl']},
 'environment_limits':{'actual':'macOS aarch64 Rust1.97.1, O3, unwind, no LTO, codegen16, one Rayon thread, jobs1, incremental0, offline; actual rustc invocations and linked artifact hashes saved.','ci_difference':'The actual Linux UPDATE job builds sparq-bench with release-fast (thin LTO/abort). This minimal unchanged-module diagnostic is not the full CLI build or exact Linux environment. Compiled mmap/dict-spill features do not assert a disk-backed runtime graph.','performance':'No query timing, throughput, heap or peak-RSS claim. Command duration includes polling.'},
 'resources':{'phase_max_new_allocated_bytes':maxgrowth,'phase_min_free_bytes':minfree,'new_limit_bytes':536870912,'minimum_free_limit_bytes':2147483648,'aggregate_limit_seconds':1200,'per_build_limit_seconds':600,'per_seed_limit_seconds':60,'within_phase_limits':maxgrowth<536870912 and minfree>=2147483648 and phase['seconds']<1200},
 'next_review_proposal':'Obtain the planned independent soundness/diagnosis review of this first-seed reproduction together with the two reduced public-API witnesses. Review a narrowly specified way to recognize or avoid reference-state lexical collapse before stateful blank-node allocation while retaining strict Sparq rebuild/in-place comparisons, full multiset probes, unknown-divergence failures and calibrated unrelated-defect controls. Do not normalize away arbitrary blank nodes, skip the seed, weaken canonical comparisons, or presume the remaining seven failures share this cause. No implementation is supplied or approved here.',
 'frozen_scope':{'included':'Actual two harnesses/locks/allowlists, needed tracked source copies, source/archive proofs, binaries, compiler/link provenance, declared protocol, raw commands/results/resources and comparisons.','excluded_regenerable':['parent-source/** (exact tracked archive; all 6443 blob identities and SHA256 values retained in parent-export-proof.json)','tmp/** (ephemeral LOAD sandbox)','external direct-5983/implementation/target (regenerable build cache; named link hashes retained)'],'no_deletion_performed':True}
}
save('report.json',report)
public={k:report[k] for k in ['author','status','source_revisions','observations','interpretation','features','environment_limits','resources','next_review_proposal']}
module=(W/'crates/sparq-bench/src/update_fuzz.rs').read_text().splitlines()
packet='# Issue 5183 — fixed original-seed diagnostic evidence\n\nNo patch is proposed in this bundle. Repository-relative source and synthetic observations only.\n\n'+json.dumps(public,indent=2)+'\n\n## Exact fixed sequence\n```sparql\n'+sequence+'\n```\n\n## Exact first failure (both revisions and CI)\n```text\n'+failure((P/'main-seed.stdout').read_text())+'\n```\n\n## Unchanged load-bearing source\n'
for a,b in [(85,128),(809,907),(1040,1205),(1233,1293),(1297,1350)]:
 packet+=f'\n### crates/sparq-bench/src/update_fuzz.rs:{a}–{b}\n```rust\n'+'\n'.join(f'{i}: {module[i-1]}' for i in range(a,b+1))+'\n```\n'
packet+='\n## Prior fixed witnesses and normative context\n\nThe frozen engine-witness contains both qualified revisions × rebuild/in-place × lexical-pair/8-only/8+9 controls. The frozen oxigraph-witness uses pinned 0.5.9 with rdf-12 and no RocksDB/HTTP features: the lexical pair yields one term, one WHERE row and one fresh blank node; controls yield one and two. No canonicalizer participates in those reduced observations. RDF 1.1 Concepts §3.3 defines literal-term identity using lexical form; SPARQL 1.1 Update §3.1.3/4.2.3 specifies fresh template blank nodes per solution. These primary sources were already verified; this phase performed no network reads.\n\nFull relevant module, actual workflow, lock/manifests, declaration-free default allowlist and unredacted local compiler diagnostics are separate evidence files. This compact packet omits the rest of the unchanged engine, the full regenerable tracked archive, host paths, raw build environment/logs and personal metadata. Prior root-verified witnesses supply the reduced runtime evidence; this packet does not claim all eight original seeds or full CI were rerun.\n'
assert '/private/' not in packet and '/Users/' not in packet
(P/'review-packet.md').write_text(packet)
# These source-only exports are additional preservation work, not a new test/build phase.
export={'utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'free_bytes':shutil.disk_usage(P).free,'source_head':git('rev-parse','HEAD'),'branch':git('branch','--show-current'),'status_porcelain':git('status','--porcelain'),'tests_or_builds_executed_during_freeze':0,'all_prior_named_commands_terminal':True,'parent_archive_regenerable_exclusion':True}
save('freeze-verification.json',export)
entries=[]
for f in sorted(P.rglob('*')):
 if not f.is_file():continue
 rel=f.relative_to(P).as_posix()
 if rel.split('/')[0] in ['parent-source','tmp'] or rel in ['manifest.json','manifest.sha256']:continue
 entries.append({'path':rel,'bytes':f.stat().st_size,'sha256':sha(f)})
save('manifest.json',{'schema':1,'files':entries,'excluded':report['frozen_scope']['excluded_regenerable'],'count':len(entries)})
(P/'manifest.sha256').write_text(sha(P/'manifest.json')+'  manifest.json\n')
assert all(sha(P/e['path'])==e['sha256'] for e in entries)
print(json.dumps({'files':len(entries),'manifest_sha256':sha(P/'manifest.json'),'packet_bytes':(P/'review-packet.md').stat().st_size,'packet_sha256':sha(P/'review-packet.md'),'included_bytes':sum(e['bytes'] for e in entries),'report_sha256':sha(P/'report.json'),'max_growth':maxgrowth,'min_free':minfree,'free_now':export['free_bytes']}))
