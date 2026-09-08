"""[GPT-6 Astra] Freeze the bounded diagnostic and its source/binary provenance."""
from pathlib import Path
import json,subprocess,hashlib,shutil,statistics as st,os
out=Path(__file__).parent;base=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees')
git=lambda wt,*a:subprocess.check_output(['git',*a],cwd=wt,text=True)
sha=lambda b:hashlib.sha256(b).hexdigest()
wt={v:base/('pr5983-bench-'+v) for v in ['candidate','disabled','main']}
frozen='9e8bdfc95c916b62550fb8c3142f804bbbfb2444';main='cf19a52c6880c496cefaca24046174902bc30958'
sourcefiles=['bench/indexed-topk/Cargo.toml','bench/indexed-topk/Cargo.lock','bench/indexed-topk/README.md','bench/indexed-topk/src/main.rs','bench/indexed-topk/src/counting.rs']
source_manifest={}
for path in sourcefiles:
 hashes={v:sha((p/path).read_bytes()) for v,p in wt.items()};assert len(set(hashes.values()))==1,(path,hashes)
 source_manifest[path]={'sha256':hashes['candidate'],'bytes':(wt['candidate']/path).stat().st_size}
 dest=out/'source'/path;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(wt['candidate']/path,dest)
assert git(wt['candidate'],'diff',frozen,'--','crates/sparq-engine','crates/sparq-core')==''
assert git(wt['main'],'diff',main,'--','crates/sparq-engine','crates/sparq-core')==''
assert not git(wt['candidate'],'status','--porcelain') and not git(wt['disabled'],'status','--porcelain') and not git(wt['main'],'status','--porcelain')
control_diff=git(wt['disabled'],'diff','3ef21cb57','HEAD','--','crates/sparq-engine/src/exec.rs');assert 'if false' in control_diff
(out/'disabled-control.diff').write_text(control_diff)
(out/'harness-registration.diff').write_text(git(wt['candidate'],'diff',frozen,'HEAD','--','bench'))
shutil.copyfile(out.parent/'metadata-candidate.json',out/'metadata-candidate.json');metadata=json.loads((out/'metadata-candidate.json').read_text());packages={p['id']:p for p in metadata['packages']};features={packages[n['id']]['name']:n['features'] for n in metadata['resolve']['nodes'] if packages[n['id']]['name'] in ['sparq-core','sparq-engine','rayon','indexed-topk-diagnostic']}
versions={'candidate':'3ef21cb57','disabled':'3861b7865','main':'f6362852e'}
provenance={}
for v,p in wt.items():
 provenance[v]={'build_head':git(p,'rev-parse',versions[v]).strip(),'current_clean_head':git(p,'rev-parse','HEAD').strip(),'worktree':str(p),'branch':git(p,'branch','--show-current').strip(),'engine_exec_blob':git(p,'rev-parse','HEAD:crates/sparq-engine/src/exec.rs').strip(),'core_store_blob':git(p,'rev-parse','HEAD:crates/sparq-core/src/store.rs').strip(),'runtime_source':'exact reviewed '+frozen if v=='candidate' else 'exact main '+main if v=='main' else 'reviewed candidate with constant-false admission only','binaries':{mode:{'path':str(out/'bins'/(v+'-'+mode)),'sha256':sha((out/'bins'/(v+'-'+mode)).read_bytes()),'bytes':(out/'bins'/(v+'-'+mode)).stat().st_size} for mode in ['timing','count']}}
measurements=json.loads((out/'measurements.json').read_text());confirmation=json.loads((out/'confirmation.json').read_text());summary=json.loads((out/'summary.json').read_text());confirm_summary=json.loads((out/'confirmation-summary.json').read_text());assert len(measurements)==72 and len(confirmation)==8 and all(x['exit_code']==0 for x in measurements+confirmation)
assert all(x['ranges_disjoint'] for x in confirm_summary)
fixtures=[]
for r in measurements:
 if r['variant']=='candidate' and r['mode']=='verify':fixtures.append(next(x for x in r['rows'] if x['kind']=='fixture'))
count_stability=[]
for r in measurements:
 if r['mode']=='count':
  samples=[x for x in r['rows'] if x['kind']=='sample'];keys=['allocs','reallocs','requested_bytes','peak_live_delta'];count_stability.append({'name':r['name'],'metrics_identical_across_three_samples':all(len({s[k] for s in samples})==1 for k in keys)})
start=json.loads((out/'build-start.json').read_text());build_elapsed=(out/'build-results.json').stat().st_mtime-start['started_unix'];assert build_elapsed<900
# Verify original semantic packet's manifest without rewriting any prior artifact.
semantic=out.parent/'semantic';sm=json.loads((semantic/'manifest.json').read_text());assert all(sha((semantic/p).read_bytes())==r['sha256'] for p,r in sm['files'].items())
report={
 'task':'PR5983 / issue6465 bounded local diagnostic after actual Opus5 xhigh source_clear_for_measurement',
 'decision':'NO_GO for admission of the current candidate; GO only for one focused decline-cost repair followed by this same matrix',
 'decision_basis':'All four adverse cases are slower than the admission-disabled control with disjoint within-process repetition ranges; a second fresh-process reversed-source-order pass confirms all four. Favorable gains and lower measured query heap do not cancel these confirmed adverse regressions. This is an engineering admission recommendation, not a new CI ratchet or security verdict.',
 'author':'OpenAI GPT-6 Astra, actual xhigh implementation/measurement runtime; no additional models or agents',
 'harness_head':git(wt['candidate'],'rev-parse','HEAD').strip(),'reviewed_runtime_head':frozen,'main_control_runtime_head':main,
 'source_provenance':provenance,'byte_identical_harness_files':source_manifest,'runtime_features':features,
 'source_note':'Final harness head differs from the measured candidate build head only in the registry status vocabulary. All harness, Cargo manifest/lockfile and runtime bytes equal those used for the stored binaries. Main/disabled registration copies are not production candidates. Historical Luke Dary / Claude Sonnet5 attribution remains intact on the unmodified reviewed branch.',
 'build':{'host':json.loads((out/'host.json').read_text()),'profile':'standalone release opt-level3, lto=false, codegen-units16, debug=false','jobs':2,'runtime_rayon_threads':1,'commands':'--release --locked --offline; count variant adds only benchmark feature count-alloc','private_target':str(out/'target'),'build_deadline_seconds':900,'last_optimized_build_completed_seconds_from_initial_attempt':build_elapsed,'result':'All six binaries built within the original deadline. Initial standalone latest-compatible resolution attempted to unpack newer local archives into the read-only user cache; two failures occurred before compilation. Reusing the full repository lock resolution offline fixed this without installation, network access or a deadline reset. All diagnostic dependency name/version pairs then matched the repository baseline.','logs':['build-candidate-timing.txt','build-candidate-aligned.txt','build-candidate-baseline.txt','build-results.json']},
 'protocol':{'fixture_points':fixtures,'initial_process_runs':72,'confirmation_process_runs':8,'matrix':'Eight fixture points × three runtime sources × verify/timing/count; reverse-order timing confirmation covers all four adverse cases × candidate/disabled','warmup_per_measured_process':2,'timing_repetitions':7,'allocation_repetitions':3,'initial_query_samples':240,'confirmation_query_samples':56,'total_measured_query_samples':296,'oracle':'Generated Rust sort on (priority, unique sequence), descending, after exact status filtering and overlay deletion; every returned subject vector compared outside measured sections','engagement':'Separate verification processes use existing EXPLAIN ANALYZE BGP trace; candidate favorable points infer indexed engagement by absence, adverse/main/disabled points show fallback. This does not replace pending explicit tracing (F4).','scope':'Whole query call includes parsing, evaluation and result construction. Result validation/formatting/destruction are outside the window. Counting and timing are separate binaries; instrumented latency is deliberately suppressed.','process_order':'Round-robin source order by fixture; reverse-order adverse confirmation in new processes. No concurrent local build during measurements. Host-wide quietness/CPU-frequency/thermal isolation was not established; load averages are saved.'},
 'results':{'initial_summary':[{k:v for k,v in r.items() if k!='metrics'} for r in summary],'confirmation_summary':confirm_summary,'allocation_repeatability':count_stability,'all_results_correct':True,'all_path_expectations_met':True},
 'F1_assessment':'No query-heap regression against the fallback controls in these favorable base/overlay fixtures. Overlay raises the candidate\'s own temporary requested-heap footprint as predicted, but it remains below fallback here. This single extent/probe/deletion density establishes no global peak-memory bound or scalability guarantee.',
 'F2_assessment':'Confirmed cost regression from attempting then declining: both near-half drained-prefix and over-cap tie fixtures regress, particularly with tombstone overlays. Preserve these fixtures as regression controls; do not publish a favorable-only performance summary.',
 'memory_limits':[
  'Counting wrapper tracks successful alloc/alloc_zeroed calls, successful reallocations and full requested new bytes. Peak live delta is requested layout bytes above the window baseline, not RSS or allocator actual usable size; it excludes allocator metadata, stack, mappings and transient realloc internals.',
  'getrusage RUSAGE_SELF reports cumulative process peak RSS (Darwin bytes). Setup, pre-query after warmup, and post-query values are separate fields. Setup/warmup can dominate later peaks; a zero later increase is not zero query memory. The pre-query difference includes warmup, result checking and output; it is not an isolated query peak.',
  'Allocator calibration passes in all three counting binaries. Isolated exact-module calibration passes; disabling either peak updating or requested-byte accounting produces the expected assertion failure (two killed controls). No Miri or external heap profiler was run.',
  'Counters are process-global, and the runtime pool is fixed to one thread; repeatability is reported rather than assumed. Timing binaries use System directly and omit the wrapper.'
 ],
 'baseline_drift_caveat':'Main and admission-disabled control medians differ by several percent in some points despite identical allocation metrics; binary/code layout and process/host variation are not disentangled. Primary attribution therefore uses the same-candidate disabled control, with main retained as a separate measured baseline. Reverse-order adverse confirmation still has disjoint ranges; do not ascribe every main/disabled timing delta to the optimizer.',
 'smallest_next_repair':{
  'first':'Reject an already oversized leading primary-key tie group before constructing the other-pattern scans and subject-id vectors, preserving the existing cap and fallback semantics. This targets the observed avoidable preparation cost without inventing a new threshold. Add a negative control for the moved admission decision and rerun this exact matrix.',
  'remaining':'The near-half drained prefix also needs an economical early rejection/scout or a cheaper probe representation. The current measurements do not justify an arbitrary cardinality cutoff or tighter failure budget; keep it a separate focused design decision. Current admission remains NO_GO until the adverse cost is addressed.',
  'not_done':'No optimizer repair, new runtime guard, admission/trace change or broader benchmark sweep was made in this spike.'
 },
 'remaining_pre_admission':[
  'F3: public-facing documentation of SPARQL-permitted tie/pagination variability across execution paths.',
  'F4: explicit indexed execution trace/counter and positive path assertions.',
  'F5: pin paths in tie/offset/view/fork semantic tests; assess Reduced and constant-object probe coverage rather than assuming all reviewer-suggested branches are reachable.',
  'Review source-context requests: term_pattern_to_term, tp_var for quoted triples, and modifier dispatch. TOP_K_ORDER_BY_THRESHOLD is1024 at this source; both measured limits are within it.',
  'Full workspace/conformance/feature admission gates and independent review of any repair/harness remain outside this local spike.'
 ],
 'validation_limits':{'registration_G3':'PASS','diff_check':'PASS','author_preflight':'Only failure remains local Bash3.2 missing mapfile in check-privacy-claims.sh:92; no gate or hook changed.','semantic_manifest_entries_unchanged':len(sm['files']),'remote_actions':'None; no push/issue/PR/comment/review/admission/dispatch/release/registry/EC2/PSS action. Read-only host sysctl metadata obtained with authorized escalation.'}
}
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n')
# Reviewable harness-only packet; prior full optimizer review stays in its frozen packet.
packet='# Local indexed top-k diagnostic decision\n\n'+json.dumps({k:report[k] for k in ['decision','harness_head','reviewed_runtime_head','main_control_runtime_head','decision_basis','F1_assessment','F2_assessment','baseline_drift_caveat','smallest_next_repair','memory_limits','remaining_pre_admission']},indent=2)+'\n\n## Harness and registration raw diff\n\n```diff\n'+(out/'harness-registration.diff').read_text()+'```\n\n## Disabled source control\n\n```diff\n'+control_diff+'```\n'
(out/'decision-packet.md').write_text(packet)
manifest={}
for p in sorted(out.rglob('*')):
 if p.is_file() and 'target' not in p.relative_to(out).parts and p.name not in ['manifest.json','freeze.json']:
  b=p.read_bytes();manifest[str(p.relative_to(out))]={'sha256':sha(b),'bytes':len(b)}
(out/'manifest.json').write_text(json.dumps({'harness_head':report['harness_head'],'reviewed_runtime_head':frozen,'files':manifest,'excluded':'task-local optimized target cache'},indent=2)+'\n')
freeze={'head':report['harness_head'],'report':str(out/'report.json'),'report_sha256':manifest['report.json']['sha256'],'packet':str(out/'decision-packet.md'),'packet_sha256':manifest['decision-packet.md']['sha256'],'manifest_sha256':sha((out/'manifest.json').read_bytes()),'file_count':len(manifest),'bytes_excluding_target':sum(x['bytes'] for x in manifest.values())};(out/'freeze.json').write_text(json.dumps(freeze,indent=2)+'\n');print(json.dumps(freeze))
