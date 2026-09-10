import pathlib, subprocess, json, hashlib, os, re, time, difflib
R=pathlib.Path('/private/tmp/sparq-pr6049/.throughput-monitor')
p=R/'direct-5183/review-followup'; w=R/'worktrees/issue5183'; f='crates/sparq-bench/src/update_fuzz.rs'
sha=lambda b:hashlib.sha256(b).hexdigest()
def put(n,v):
 q=p/n;q.parent.mkdir(parents=True,exist_ok=True);q.write_text(json.dumps(v,indent=2)+'\n' if not isinstance(v,str) else v)
def git(*a):return subprocess.check_output(['git',*a],cwd=w)
def portion(s,start,end):return s[s.index(start):s.index(end,s.index(start))]
head=git('rev-parse','HEAD').decode().strip();parent=git('rev-parse','HEAD^').decode().strip()
assert head=='fe3284199db0831f353d2ae401b8c69472764904' and parent.startswith('787aab5c')
assert git('status','--porcelain')==b''
s=git('show',head+':'+f).decode();before=git('show',parent+':'+f).decode();base=git('show','4595388d:'+f).decode()
assert (w/f).read_text()==s and (p/'before.rs.txt').read_text()==before
# Confirm the control used the exact final source, with only the named mutant.
m=(p/'controls/partial-canonical-pool-overlap/update_fuzz.rs').read_text()
assert m.replace('4 => format!("{}", rng.below(40)),','4 => format!("{}", gen_canonical_integer(rng)),')==s
put('final.rs.txt',s)
put('delta.diff',git('diff',parent,head,'--',f).decode());put('full.diff',git('diff','4595388d',head,'--',f).decode())
put('commit.txt',git('show','-s','--format=fuller',head).decode());put('worktree-status.txt',git('status','--porcelain').decode())
blocks={
 'nquads_line':('fn nquads_line(', 'fn sparq_nquads('),
 'comparable':('fn comparable(', '/// Rewrites `t`'),
 'oxigraph_normalized_term':('fn oxigraph_normalized_term(', '// [GPT-6 ASTRA] Quad'),
 'oxigraph_normalized':('fn oxigraph_normalized(', '/// The lines on exactly'),
 'one_sided':('fn one_sided(', '/// The outcome of comparing'),
 'Verdict':('enum Verdict {', '/// Compares two snapshots'),
 'allowlist':('struct UpdateDivergenceAllowlist {', '// ── entry point'),
}
equiv={n:{'before_sha256':sha(portion(before,*v).encode()),'final_sha256':sha(portion(s,*v).encode()),'byte_identical':portion(before,*v)==portion(s,*v)} for n,v in blocks.items()}
assert all(v['byte_identical'] for v in equiv.values())
a=portion(before,'fn compare(','// ── probe SELECTs');b=portion(s,'fn compare(','// ── probe SELECTs')
assert a[:a.index('        let normalized =')]==b[:b.index('        let normalized =')]
assert a[a.rindex('\n            (Err(e), _) | (_, Err(e)) => {'):]==b[b.rindex('\n            (Err(e), _) | (_, Err(e)) => {'):]
# The full-record helper changes only syntax; retain both actual complete bodies.
record_before=portion(before,'fn record_lexical_terms(', 'struct NormalizedSnapshot')
record_final=portion(s,'fn record_lexical_terms(', 'struct NormalizedSnapshot')
put('record-helper-before.rs.txt',record_before);put('record-helper-final.rs.txt',record_final)
put('helper-equivalence.json',{'comparison_parent':parent,'final':head,'unchanged_helpers':equiv,'compare_initial_prologue_and_raw_count_guard_identical':True,'compare_outer_normalization_error_and_final_Differs_identical':True,'record_helper':'Same parse guard, one BTreeMap insert per parsed integer, mismatch-only error, recursive call and blank-node handling; 2024 let-chain becomes 2021 nested if/match. Full before/after bodies frozen. Full suite and unchanged integer-injectivity controls exercise it.','main_base_qualification':'4595388d is the original whole-patch base; its comparator is intentionally changed by parent787. No semantic equivalence to that base is claimed for the complete patch.'})
commands=json.loads((p/'commands.json').read_text()); resources=json.loads((p/'resources.json').read_text());controls=json.loads((p/'control-results.json').read_text());artifact=json.loads((p/'test-artifact.json').read_text())
oldprov=json.loads((R/'direct-5183/implementation/compiler-provenance.json').read_text());deps=[]
for old in oldprov['direct_extern_dependencies']:
 q=pathlib.Path(old['path']);h=sha(q.read_bytes());assert h==old['sha256'];deps.append(dict(old,still_matches_prior=True))
put('compiler-provenance.json',{'toolchain':oldprov['toolchain'],'actual_test_rustc_argv':json.loads((p/'test-rustc-argv.json').read_text()),'source_sha256':sha(s.encode()),'binary_sha256':sha((p/'candidate-tests-binary').read_bytes()),'direct_extern_dependencies':deps,'feature_proof':'feature-edition-proof.json','edition_correction':'Original harness used2024; this actual rustc invocation uses2021, matching inherited workspace edition.','qualification':'Exact module imported by minimal native harness; not full sparq-bench workspace compilation or authoritative Linux CI. Existing engine/core/canon dependencies retain hash-verified prior feature configuration.'})
assert sha((p/'candidate-tests-binary').read_bytes())==artifact['export_sha256']
put('test-summary.json',{'suite':'20 passed;0 failed;0 ignored (exact module, serial, edition2021)','full_stdout_sha256':sha((p/'full-suite.stdout').read_bytes()),'clippy':'cargo clippy --release --locked --offline -j1 --tests -- -D warnings; exit0','marker_strict_ambient':'Exact candidate marker test passes with empty ambient allowlist; local explicit allowlist reaches adjudication.','control_results':[{k:v for k,v in c.items() if k not in ('build_argv',)} for c in controls],'fault_injection_limit':'Third comparable() call is deliberately injected to fail after both raw dataset calls succeed. Positive uses final branch; negative has identical fault/test and old error-only branch. This establishes diagnostic preservation on that branch, not a naturally observed canonicalizer failure.','prior_controls':'No repeat of seven parent controls: generator partition/LOAD draw policy, record recursion, duplicate refusal, count/blank-node guards, and marker detection conditions unchanged; diagnostic formatting, syntax and test oracles changed. Current20-test suite plus focused follow-up controls passed; prior control outputs remain frozen separately.'})
light=json.loads((p/'lightweight-checks.json').read_text())
put('source-provenance.json',{'head':head,'parent':parent,'base':'4595388de9e389f5369d63828fbf90cfc16b62d9','changed_paths':[f],'source_sha256':sha(s.encode()),'parent_source_sha256':sha(before.encode()),'base_source_sha256':sha(base.encode()),'source_hash_bound_to_control_baseline':True,'actual_model':'GPT-6 Astra (xhigh)','coauthor':'GPT-6 Astra <noreply@openai.com>','clean':True,'delta_stat':git('diff','--stat',parent,head).decode().strip(),'prior_implementation_manifest_sha256':'76c1012bf2fa7d784006dd70fe075559aebc59ff06addca6550b783d68600421','prior_review_result_sha256':sha((R/'direct-5183/admission/opus-injective-patch-result.json').read_bytes())})
report={'issue':5183,'head':head,'parent':parent,'base':'4595388de9e389f5369d63828fbf90cfc16b62d9','author':'GPT-6 Astra (xhigh)','decision':'Focused follow-up complete for independent delta review; no publication/admission claim.','changes':['Replace two new Rust2024 let chains with equivalent Rust2021 constructs. Workspace Cargo.toml edition2021 and sparq-bench inheritance remain unchanged.','Keep original ca/cb one-sided diagnostics plus accurate reason/counts in the new blank-node and normalized comparable error branches.','Independently assert20/80 integer pool boundaries and spelling for every parsed generated integer leaf, including nested objects and LOAD; preserve existing40/400/200 test suites.','Use explicit local marker-test allowlist and prove independence from empty ambient registry.','Two existing test expressions received rustfmt wrapping-only changes.'], 'validation':'test-summary.json','edition_gap':'Previous20/20 under edition2024 did not prove production compatibility. Unchanged787 source fails under the same2021 compiler command with two let-chain diagnostics; finalsource passes20tests and Clippy. This is a material corrected gap.','helper_equivalence':'helper-equivalence.json; parent787 tofinal: normalizers, Verdict, allowlist, one_sided and comparable byte-identical; record only syntax; compare prologue/raw guard/normalization error/finalDiffers preserved. Complete patch intentionally differs from base459.','review_dispositions':{'raw_count_guard':'Not dead: initial guard runs only when ca==cb; new lexical-path guard runs when ca!=cb. Neither removed.','omitted_context':'Prior reviewer omission caveat valid: complete source was frozen separately, but not fully supplied. New packet includes complete compare prologue/finalDiffers, Verdict, allowlist, both normalizers, record helper, relevant callers/tests and edition/toolchain excerpts. It does not claim prior packet had complete source.','unchanged_guards':'Seven prior controls are preserved rather than rerun; focused current controls and full20tests cover changed syntax/diagnostics/oracles.','resource_scope':'No workspace/engine/dependency rebuild expansion, no other original seed execution.'}, 'lightweight':light,'preflight_limit':'exit1 solely privacy-claims shell mapfile unavailable under installed Bash3; G1/G2/G6/guard-untested pass; no-perf-numbers/readme-template correctly skip unmatchedpaths. Not weakened or bypassed. Linux gate remains required.','recorded_setup_corrections':['Export-only helper equivalence substring assertion matched inner arm; corrected to exact outer-arm line. Read-only ps denied by sandbox; terminal receipts used without retry or alternate process inspection.','Exact compactTOML edition replacement initially failed an assertion before build; follow-on runner missing exit2; corrected and preserved setup receipt.','SystemPython3.9 lacks tomllib; first metadata proof script failed, but subsequent compile started because initial shell lacked fail-fast. Actual compile uses2021; laterproof derives edition fromCargoJSON and sectionscopedsource.','Unfilteredmetadata had42 foreign-platform dependencies; native-filtered proof matches all102 original native dependency feature sets. No package/version change, dependency install or outcome retry.'], 'limits':['Native minimal exact-module tests are not full sparq-bench/fullworkspace or LinuxCI proof.','Fault injected normalized error demonstrates formatting preservation only.','Narrower generated reference domain and historical Sparq-only regression remain as parent; no Sparq engine defect fixed. Seven other original advancing-window failures remain unknown; changed-generator same-seed runs do not resolve them.'],'commands_terminal':True,'next_step':'Root reviews frozen focused packet with actual independent Opus, then owns supported CI/publication.'}
put('report.json',report)
# Exact source context with complete load-bearing functions and repository-relative labels.
excerpts=[('complete comparison helpers', portion(s,'/// Whether a snapshot mentions','// ── probe SELECTs')),
 ('nquads_line',portion(s,'fn nquads_line(','fn sparq_nquads(')),
 ('PROBES',portion(s,'const PROBES:', '/// Renders one probe')),
 ('production apply_sequence',portion(s,'fn apply_sequence(', '// ── adjudicated-divergence')),
 ('complete allowlist',portion(s,'/// The adjudicated `update-*`', '// ── entry point')),
 ('test allowlist helper',portion(s,'    fn allowlist()', '    /// The per-PR BLOCKING')),
 ('all new and changed semantic tests',s[s.index('    // [GPT-6 ASTRA] These checks'):]),
 ('generator integer pool',portion(s,'const CANONICAL_INTEGER_VALUES:', 'fn gen_subject('))]
context=''
for title,body in excerpts:
 start=s.index(body);line=s[:start].count('\n')+1
 context+='\n### '+title+' — '+f+':'+str(line)+'\n\n```rust\n'+body.rstrip()+'\n```\n'
put('complete-function-context.md',context)
configs=''
for rel,start,end in [('Cargo.toml','[workspace.package]','[workspace.dependencies]'),('crates/sparq-bench/Cargo.toml','[package]','[dependencies]')]:
 text=(w/rel).read_text(); body=portion(text,start,end);configs+='\n### '+rel+'\n\n```toml\n'+body+'```\n'
tool=(w/'rust-toolchain.toml').read_text(); vals='\n'.join(l for l in tool.splitlines() if l.startswith(('[toolchain]','channel =','profile =','components =','targets =')))
configs+='\n### rust-toolchain.toml — actual configuration (explanatory comments omitted)\n\n```toml\n'+vals+'\n```\n';put('edition-toolchain-context.md',configs)
# Record the old edition compiler diagnostics without private source paths.
errors=[]
for line in (p/'old-source-edition2021-build.stderr').read_text().splitlines():
 try:d=json.loads(line)
 except ValueError:continue
 if d.get('level')=='error':errors.append({'message':d['message'],'source_lines':[v.get('line_start') for v in d.get('spans',[]) if v.get('is_primary')]})
put('edition-negative-summary.json',errors)
public_report={k:v for k,v in report.items() if k!='lightweight'}
public_report['validation_commands']=[{k:v for k,v in x.items() if k in ['name','exit','seconds']} for x in commands]
public_report['checks']=[{k:v for k,v in x.items() if k in ['name','exit']} for x in light]
packet='# Issue5183 focused follow-up for actual independent review\n\nFinal '+head+'; parent '+parent+'; originalbase4595388d. Actual author: GPT-6 Astra (xhigh).\n\nReview the delta and corrected Rust2021 proof. Prior scoped review approved787 subject to low findings; initial packet omitted unchanged bodies and its2024 test harness missed a real compatibility defect. Full original source existed in separate frozen evidence. The functions below close that packet gap.\n\n## Structured observations and limits\n\n```json\n'+json.dumps(public_report,indent=2)+'\n```\n\n## Exact delta\n\n```diff\n'+(p/'delta.diff').read_text()+'```\n'+configs+context
packet+='\n## Initial-parent record helper (final version is above)\n\n```rust\n'+record_before+'```\n\n## Old-source Rust2021 compiler negative\n\n```json\n'+json.dumps(errors,indent=2)+'\n```\n'
packet+='\n## Actual suite stdout\n\n```text\n'+(p/'full-suite.stdout').read_text()+'```\n\n## Executed controls (structured; raw named diagnostics kept separately)\n\n```json\n'+json.dumps(json.loads((p/'test-summary.json').read_text()),indent=2)+'\n```\n'
for name in ['partial-canonical-pool-overlap','omit-blank-node-dataset-details','restore-ambient-marker-allowlist','normalized-error-fault-positive','normalized-error-old-detail-control']:
 packet+='\n### '+name+' exact scratch delta\n\n```diff\n'+(p/'controls'/name/'delta.diff').read_text()+'```\n'
packet+='\n## Context boundaries\n\nThis supplement supplies complete named comparison, normalization, record, allowlist and apply_sequence bodies, all new semantic tests, and exact old record-helper syntax. Unchanged generator operation constructors, LoadSandbox implementation, engine/parser/canon internals and old legacy tests are not repeated here; their full source remains in the prior immutable evidence and final.rs.txt. This is a native exact-module result, not full crate/workspace or CI approval. No security, engine-correctness or performance claim is added.\n'
assert '/private/tmp/' not in packet and '/Users/' not in packet and '/home/' not in packet
put('review-packet.md',packet)
# Final concrete resources and no active scoped process observation, without unrelated process disclosure.
def allocated(q):
 return sum(x.stat().st_blocks*512 for x in q.rglob('*') if x.is_file() and not x.is_symlink())
initial=json.loads((p/'initial-resource.json').read_text());target=R/'direct-5983/implementation/target'
free=os.statvfs(p).f_bavail*os.statvfs(p).f_frsize
final_resources={'initial':initial,'final_free_bytes':free,'final_target_allocated':allocated(target),'final_evidence_allocated':allocated(p),'max_monitored_new_allocated':max(x['new_allocated'] for x in resources),'min_monitored_free_bytes':min(x['free_bytes'] for x in resources),'monitored_command_seconds':sum(x['seconds'] for x in commands),'lightweight_seconds':sum(x['seconds'] for x in light),'limits':{'seconds':300,'new_allocated':134217728,'free':2147483648}}
final_resources['final_new_allocated']=max(0,final_resources['final_target_allocated']-initial['initial_target_allocated'])+final_resources['final_evidence_allocated']
assert final_resources['final_new_allocated']<134217728 and free>2147483648
put('final-resources.json',final_resources)
put('terminal-receipt.json',{'commands':len(commands),'all_returned':all('exit' in x for x in commands),'managed_sessions':'Controls exec57826 returned exit0; all17 monitored command receipts have terminal exit codes. Export exec97690 exited1 on denied ps. No build/test is pending in managed sessions.','process_inventory_limit':'Read-only ps was denied by filesystem/process sandbox; no retry or alternate route attempted. Independent OS process census not established.','completion_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())})
print(json.dumps({'head':head,'packet_bytes':len(packet.encode()),'packet_sha256':sha(packet.encode()),'resources':final_resources},indent=2))
