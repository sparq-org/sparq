"""[GPT-6 Astra] Freeze a whole-change review packet and reproducible evidence manifest."""
from pathlib import Path
import subprocess,json,hashlib,re,shutil
out=Path(__file__).parent;stage=out.parent/'implementation';root=Path.cwd()
git=lambda *a:subprocess.check_output(['git',*a],text=True)
head=git('rev-parse','HEAD').strip();base='cf19a52c6880c496cefaca24046174902bc30958';stagehead='803bb795201782551d984221a48b32ed391f7d9c'
assert head=='9e8bdfc95c916b62550fb8c3142f804bbbfb2444' and not git('status','--porcelain')
no_comments=lambda s:'\n'.join(x for x in s.splitlines() if not x.lstrip().startswith('//'))
assert no_comments(git('show',stagehead+':crates/sparq-engine/src/exec.rs'))==no_comments(git('show',head+':crates/sparq-engine/src/exec.rs'))
mut=json.loads((out/'mutation-report.json').read_text());assert mut['counts']=={'KILLED':8,'SURVIVED':0,'INVALID_OR_ERROR':0}
validation={}
for name in ['default','no-default','compact-index','final-default']:
 text=(out/(name+'.txt')).read_text();m=re.search(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',text);assert m and tuple(map(int,m.groups()))==(26,0,0),name
 validation[name]={'passed':26,'failed':0,'ignored':0,'log':name+'.txt'}
assert 'mapfile: command not found' in (out/'final-preflight.txt').read_text()
report={
 'task':'PR5983 bounded semantic/resource extension; issue6465 tracks stage-one confirmed findings',
 'status':'Clean local whole-change candidate for independent Opus5 xhigh review, not publication/admission or a measured performance claim',
 'model':'OpenAI GPT-6 Astra, actual xhigh runtime; no other model sessions or agents',
 'head':head,'base':base,'local_origin_main_observed':git('rev-parse','origin/main').strip(),'stage1_head':stagehead,'branch':git('branch','--show-current').strip(),'worktree':str(root),
 'provenance':{'source_pr5983':'59bfc6c44b25a4e107d5d957ca3287ba0bc22c1a','historical_mergebase':'fbbd310326f1f083c33a13deee19f95cb96c55f6','historical_author':'Luke Dary <ldary@redhat.com>, historical Claude Sonnet5 coauthorship retained in import checkpoint d3ee2b1b6059008e185ef946357b6cf77c21341a','astra':'Three-way mechanical import, stage-one tests/guards and this semantic extension honestly attributed; original PR timing claims remain unverified','semantic_commits':['eae19f2b5c571a3fed83f60ce63a96ef215348e1',head]},
 'scope':{'whole_change':git('diff','--stat',base,head).strip(),'files':git('diff','--name-only',base,head).splitlines(),'semantic_delta':'Seven additional tests, permutation-aware engagement assertion, test-contract prose and a five-line tie-soundness clarification. No executable runtime change from stage1 (verified after excluding full-line comments). No new runtime guard added.','outside_scope_untouched':['PreparedGraphApplier','transaction retry helpers','dependency/version metadata','public API','main bind_join changes','CI/gate/approval files','other branches/shared checkout','registry/release/EC2/PSS']},
 'semantic_evidence':[
  'Total-order DESC primary/secondary keys: eight OFFSET/LIMIT windows including zero, primary-group boundary, tail and beyond-end; compare full ORDER BY window.',
  'Exact full-key ties: five windows check valid member ids, distinct multiplicity, key value and permitted count; no assertion of unspecified stable tie order or particular surviving subset.',
  'Three negative/mixed/typed-lexical fixtures: exact full-order prefix plus actual fallback trace. Inline-guard deletion is killed by admission assertion on first fixture; do not claim that control proved wrong numeric results on all fixtures.',
  'StoreDefault and Empty default views; visible GRAPH subquery; hidden/absent GRAPH and FROM; plain API after scope release. Empty-default-guard deletion actually leaks one row where zero is required.',
  'Fork overlay tombstone plus new task insertion; immutable original snapshot; four k windows against independent Rust-sort oracle; compaction preserves answers.',
  'Armed false cancellation flag and future deadline require actual BGP fallback. Already-set cancellation and already-expired deadline error deterministically; later unlimited query succeeds. No mid-query cancellation-latency guarantee is claimed.',
  'Restricted compact-index executes all26 tests, explicitly asserts fallback when this star fixture lacks PSO; default/no-default feature builds positively assert indexed engagement.'
 ],
 'validation':{'focused':validation,'existing':json.loads((out/'existing-results.json').read_text()),'unique_passed_tests':51,'existing_ignored':'One pre-existing dataset-view microbenchmark bench_view_vs_from_named_copy is ignored; not counted as passed.','baseline_executions':'26 default +26 no-default +26 compact-index;25 additional existing tests. Final comment-only head reruns26 default. Mutation baselines and mutant executions are reported separately.','mutations':mut,'mutant_notes':'Eight controls each executed all26 tests: repeated deleted/inert, budget deleted/inert, multivalue guard deleted, indexed path disabled, inline guard deleted, empty-default guard deleted. No survivor or invalid/compile-only kill. Budget deletion/inert each fail3 tests; all others fail1. Controls ran eae19f2b5; final exec differs only in five documentation lines. Exact source restored before final26-pass run.','cargo':'Pinned installed rustc1.97.1/cargo1.97.1, --locked --offline, two build jobs, RAYON_NUM_THREADS=2, test-threads=1, debug info0, task-local stage1 target reused. No installations.','feature_limit':'--no-default-features disables engine parallel/regex/digest defaults, but its existing core dev-dependency still enables core parallel/mmap/dict-spill. This is not a wholly single-threaded-core or wasm build claim.','preflight':'Final preflight exits1 only on local Bash3.2 missing mapfile in scripts/check-privacy-claims.sh:92. Other applicable checks ran. No hook/gate bypass or script changes.','diff_check':'PASS','stage1_integrity':json.loads((out/'stage1-integrity.json').read_text())},
 'limits':[
  'No latency, throughput, CPU, allocation-count or peak-memory measurement. Incidental EXPLAIN timing text is not benchmark evidence. No O(k) or PostgreSQL-gap claim.',
  'Upfront full seed inline validation and per-probe subject-id vectors remain linear. Overlay/compressed scans may allocate. Selectivity/tie/failure guards can spend work before fallback. These costs require measurement before a performance claim.',
  'No new resource arithmetic: every armed budget declines to existing accounting/cancellation. The unlimited shortcut still performs unbudgeted setup allocations.',
  'Full-key tie results can differ from stable fallback input order while remaining valid SPARQL. Callers requiring a deterministic subset must supply a distinguishing ORDER BY key.',
  'W3C fixture checkout and conformance binary are absent in this isolated checkout/cache. No fetch, conformance run, full-workspace build/clippy/test, wasm, ZK, algebra-rewrite-specific, mmap/compressed-file or release-feature test is claimed.',
  'The packet includes complete critical executable callers/helpers and raw diff. It does not recursively duplicate the entire unchanged expression evaluator, substrate comparator, parser, storage serialization or optional feature implementation. Those boundaries remain normal whole-project review/conformance responsibilities.',
  'No independent review, remote branch/issue/comment mutation, admission, dispatch, live CI operation or infrastructure change.'
 ],
 'next_step':'Root obtains actual independent Opus5 xhigh whole-change review. Resolve concrete blockers before measurement or publication. Then prepare one reproducible local benchmark using the existing registry: compare exact candidate, same candidate with indexed admission disabled, and current main on identical generated single-valued star data. Vary depth/k/tie density/selectivity/drained prefix; report full raw data, features, host, repetitions and controlled threads, including setup/allocation costs. Full conformance/workspace gates remain required before admission.'
}
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n')
# Embed key historical red/green controls unchanged, so no tool access is needed to assess T1/T2.
prior=['t1-raw.txt','t1-main-fallback.txt','resource-raw.txt']
for name in prior:shutil.copyfile(stage/name,out/('stage1-'+name))
intro=f'''# Whole-change independent review request: PR5983 narrow indexed top-k recovery

Frozen head `{head}` on `{base}`. Actual implementation runtime: GPT-6 Astra xhigh. Historical Luke Dary / Claude Sonnet5 source is preserved and attributed; timing claims are unverified. This is a local candidate, not reviewed, measured, published or admitted.

Please assess correctness and resource admission of the entire two-file change, including stage-one repairs (issue6465), then this test extension. Treat implementation claims below as claims to check. Return blocking findings with exact source locations and concrete counterexamples; distinguish a known conformance/performance validation limit from an observed defect. Do not infer a performance gain from the name or historical comments.

The whole raw diff below contains the complete new indexed function and complete test file. The companion context carries full critical executable functions/callers from this exact Git head. Only standalone full-line comments are omitted from the context copy inside this packet to reduce repeated prose; `source-context.md` preserves the complete unabridged source excerpts with Git blobs and ranges. No statements are omitted. The budget excerpt explicitly omits unchanged test submodules. Deeper unchanged evaluator/type/parser/storage code is not a recursive source audit; report a specific missing dependency if a decision needs it.

'''
packet=intro+'## Evidence, contract and limitations\n\n```json\n'+json.dumps(report,indent=2)+'\n```\n\n## Raw whole-change diff\n\n```diff\n'+(out/'whole-change.diff').read_text()+'```\n\n## Critical unchanged caller/helper context\n\n'+(out/'source-context-compact.md').read_text()+'\n## Historical executed red/green evidence\n\n'
for name in prior:packet+=f'### {name}\n\n```text\n'+(stage/name).read_text()+'```\n\n'
packet+='## Final exact-head focused execution\n\n```text\n'+(out/'final-default.txt').read_text()+'```\n'
(out/'review-packet.md').write_text(packet)
manifest={}
for path in sorted(out.rglob('*')):
 if path.is_file() and path.name not in ['manifest.json','freeze.json']:
  data=path.read_bytes();manifest[str(path.relative_to(out))]={'sha256':hashlib.sha256(data).hexdigest(),'bytes':len(data)}
(out/'manifest.json').write_text(json.dumps({'head':head,'base':base,'files':manifest},indent=2)+'\n')
freeze={'head':head,'packet':str(out/'review-packet.md'),'packet_sha256':manifest['review-packet.md']['sha256'],'packet_bytes':manifest['review-packet.md']['bytes'],'manifest_sha256':hashlib.sha256((out/'manifest.json').read_bytes()).hexdigest(),'file_count':len(manifest),'clean_worktree':not bool(git('status','--porcelain'))}
(out/'freeze.json').write_text(json.dumps(freeze,indent=2)+'\n')
print(json.dumps(freeze))
