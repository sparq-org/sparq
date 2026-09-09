# [GPT-6 Astra] Freeze the bounded evidence-completion phase, not a production patch.
from pathlib import Path
import json,hashlib,collections,shutil,time
E=Path(__file__).resolve().parent;S=E/'source/sparq';W=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue6475')
H=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
f='crates/sparq-zk/src/vc_bridge_sd.rs';(S/f).parent.mkdir(parents=True,exist_ok=True);shutil.copy2(W/f,S/f)
matrix=json.loads((E/'rust-matrix.json').read_text());suite=json.loads((E/'suite-results.json').read_text());summary=json.loads((E/'suite-summary.json').read_text());budget=json.loads((E/'rust-budget-cases.json').read_text())
assert len(matrix)==12 and all('value' in x['result'] for x in matrix)
values={label:{x['result']['value'] for x in matrix if x['labeling']==label} for label in ['original','renamed']}
assert len(values['original'])==len(values['renamed'])==1 and values['original']!=values['renamed']
matsummary={'cases':12,'accepted_under_Rust_defaults':12,'quad_order_invariant_for_this_pair':True,'relabel_invariant_for_this_pair':False,'canonical_outputs':{k:next(iter(v)) for k,v in values.items()},'budget_threshold':{'each_renaming_limits0through3':'HndqCallLimitExceeded(limit)','limit4':'success','default':'success at global limit4000'},'generalization':'Only this concrete pair tested; no general order-invariance proof.'}
(E/'matrix-summary.json').write_text(json.dumps(matsummary,indent=2)+'\n')
rows=[]
for runtime,limit in [('rust','default'),('js','default'),('js','4000')]:
 xs=[r for r in suite if r['runtime']==runtime and r['budget']==limit]
 rows.append({'runtime':runtime,'budget':limit,'types':{kind:{'total':sum(r['type']==kind for r in xs),'pass':sum(r['type']==kind and r['pass'] for r in xs)} for kind in sorted({r['type'] for r in xs})},'skips':0,'external_timeouts':sum('external_timeout_seconds' in r['result'] for r in xs),'nonJSON_errors':sum('harness_error' in r['result'] for r in xs)})
(E/'suite-counts.json').write_text(json.dumps(rows,indent=2)+'\n')
consumers=[
{'path':'crates/sparq-canon/src/lib.rs','lines':'112–114,310–352,365–412','role':'Public dataset canonical bytes, digest of exact canonical bytes, and issuer maps delegate through shared bridge to rdf-canon. SHA256 default; parameterized profiles share algorithm.','input':'General RDF1.1 quads including blank graph names; saved shape reachable.'},
{'path':'crates/sparq-canon/src/concept.rs','lines':'272–279','role':'Opt-in concept record digest/URN verification hashes canonical record bytes through digest_quads_with.','input':'Caller-supplied quads; canonical-byte/error contract matters.'},
{'path':'crates/sparq-wasm/src/canon.rs; js/src/dataset.ts','lines':'31–32;283,397–398','role':'Optional wasm canonicalizeNQuads forwards errors as JsError; RDF/JS Dataset.equals compares canonical strings and toCanonical returns them.','input':'General datasets; actual exported path affected. Native/wasm execution not rerun here.'},
{'path':'crates/sparq-vc/src/suite.rs','lines':'309–320','role':'hash_data signs/verifies SHA256(canonical proof configuration)||SHA256(canonical document).','input':'Per-document triples only; this exact mixed-graph counterexample is not demonstrated reachable.'},
{'path':'crates/sparq-zk/src/canon.rs; commit.rs; dual_leaf.rs','lines':'23–25;239–258;648–680','role':'Reexported graph canonicalization defines canonical triple order, leaf order and Poseidon2 commitment inputs.','input':'Per-graph triples; current dataset counterexample not a demonstrated exploit or reachability result.'},
{'path':'crates/sparq-zk/src/trace.rs','lines':'82–107','role':'Issued canonical labels map source triples to committed canonical leaf indices.','input':'Per-graph triples; output/map agreement must be preserved.'},
{'path':'crates/sparq-zk/src/vc_bridge.rs; vc_bridge_sd.rs','lines':'318–340;147–148','role':'Canonical proof/document strings feed signature hashData and selective-disclosure host verifier interface.','input':'Triples; same qualification as VC above.'},
{'path':'crates/sparq-difftest/src/iso.rs','lines':'61,266–271','role':'Separate direct rdf-canon caller for differential isomorphism checks, bypasses Sparq bridge with explicit100000-call budget.','input':'Comparison encoding; a Sparq seam-only repair would not repair this independent algorithm consumer.'},
{'path':'crates/sparq-bench/src/update_fuzz.rs; crates/sparq-canon/src/rdf12.rs','lines':'774–784;195–197,318–332','role':'Update-fuzz comparison uses the separate nonstandard RDF1.2 implementation via ground-term guard, not upstream rdf-canon.','input':'Separate implementation; standard dependency repair will not automatically change it. This phase did not replay the failure there.'}
]
(E/'consumer-inventory.json').write_text(json.dumps({'scope':'Tracked source references and named aliases, with test/example matches separated by manual source inspection; not a dynamic whole-workspace call graph or external consumer census.','queries':['rg -n sparq_canon::|use sparq_canon|rdf_canon:: crates --glob *.rs --glob !**/tests/**','rg -n canonicalize_triples|canonicalize_graph_content|canonical_nquads crates --glob *.rs --glob !**/tests/**','rg -n canonicalize_nquads|canonicalizeNQuads --glob *.ts --glob *.js --glob *.mjs'],'consumers':consumers},indent=2)+'\n')
options=[
{'option':'Bounded comparison of complete equal-hash candidate outcomes at the shared canonicalizer','assessment':'Most directly aligned with preserving the existing success/invariance property, but still a design proposal. Preserve all distinct-hash behavior and genuine symmetry. When tied complete candidate issuers produce distinct full canonical documents, a deterministic document-order choice could remove input-order dependence. Partial issuers/ties across later groups need a sound completion strategy under the existing budget; do not improvise a local original-label tie break.','compatibility':'This is a canonical-output extension for an underdetermined case, not a proven existing RDFC1.0 requirement. Must test byte preservation over this baseline, property matrix, issuer/output consistency, poison limits, then obtain actual patch review. No source is approved or changed.'},
{'option':'Detect divergent complete tied outcomes and return a typed ambiguity error','assessment':'Could prevent inconsistent canonical bytes and preserve genuine symmetric outputs; requires complete-outcome proof and bounded work. A blanket equal-hash rejection is disproved as a safe policy by successful symmetric W3C controls.','compatibility':'Changes currently accepted input to error. Current Sparq property calls expect success, so this alone would NOT repair the gate. No relaxation, exception or quarantine of that property is authorized.'},
{'option':'Replace or switch canonicalizer','assessment':'Not evidence-supported as a small fix: JS5 shares the underdetermination and its default budget also rejects18 positive W3C fixtures. A separate complete canonical-labelling algorithm would be larger and could change standard bytes.','compatibility':'Do not switch implementation merely to make this one relabeling order pass.'}
]
report={'issue':6475,'author':'OpenAI GPT-6 Astra, actual inherited xhigh lane','scope':'Evidence completion only; no production edit, commit or remote mutation','completed':['B1 source seam/pins/property/consumer inventory','B5 default/global budget and exact0..4 probes','B7 complete12-case Rust matrix','All86 packaged W3C manifest entries on unmodified Rust and JS5 in two fixed budget modes','B6 brief source-only issuer-order assessment'],
'B1':{'main':'e53464c73f31f7aca800f3867ac054c36408e346','pr_head':'86b8dfa232ad7d315dd56df28759ac5fca7b365c','merge_group':'2adb86d91345deb51e95b11af4e2b5572343c236','failed_source_job':'run34409424033/job102661090898, opt-in group g20, cargo test -p sparq-canon --features concept','property':'canonical_output_invariant_under_bnode_relabeling at tests/proptest_canon_determinism.rs:370–384;256cases unchanged; materialize preserves quad order and component roles with injective label map.','seam':'canonicalize_quads(lib.rs310) -> serialize_quads/default435 or feature-lowcopy -> parse_02(495) -> rdf_canon::canonicalize_quads; default HNDQ budget retained.','dependency':'Cargo.toml declares compatible version constraints0.15.3/0.2.4/0.1.8, not exact-equals pins. Cargo.lock locks registry rdf-canon0.15.3,oxrdf0.2.4,oxttl0.1.8. Sparq native models use oxrdf0.3.3/oxttl0.2.3. No relevant source patch override found in root Cargo.toml/.cargo.','identity':'Three seam/test/manifest files byte-identical to frozen failing CI source.'},
'B5':{'default':4000,'semantics':'SimpleHndqCallCounter increments a single global counter on entering hash_n_degree_quads; counter>limit errors. None/default selects4000; the public API uses Simple, not PerNode. This is a call bound, not a deadline/strict full-work bound.','error':'CanonicalizationError::HndqCallLimitExceeded(usize); Sparq maps it to CanonError::Canonicalization(String), losing enum detail but propagating failure.','counterexample':'Both renamings require exactly4 HNDQ entry calls: limits0..3 error,4/default succeed. An ambiguity rejection is demonstrably a new rejection for this accepted input.','accepted_set_limit':'Current acceptance characterized for this fixed matrix and all packaged fixtures, not an exhaustive mathematical set of all inputs.'},
'B6':{'finding':'Source restores temporary issuer order by BTreeMap keyed on bN strings, although issuer map is HashMap and issuance uses integer counter. At11+ labels lexical b10 precedes b2, differing from chronological issuance.','status':'Independent source lead warrants a dedicated small fixture if root prioritizes it. No11+ temporary-issuer counterexample was constructed in this phase; not asserted as the cause of6475 or a demonstrated output failure. All packaged baseline fixtures still passed.'},
'N2':'Pure bijective relabeling with the quad sequence held fixed preserves abstract first-appearance order. No fabricated relabeling was used to claim otherwise; JS order dependence remains the distinct test.',
'matrix':matsummary,'W3C_summary':summary,'W3C_counts':rows,
'validation_limits':['Custom baseline drivers call the unmodified production APIs; upstream Rust unit-test module and Sparq engine/workspace were not built.','Complete means86entries in the immutable packaged rdf-canon0.15.3 manifest, not a claim about a freshly downloaded latestW3C suite.','JS default mode retains work-factor1 but adds an explicit1s case timeout; diagnostic mode fixes4000 iterations and1s. Outer processes are bounded2s. No timeouts, unknown types or skipped entries observed.','No broad randomized differential corpus, arbitrary-input correctness proof, new algorithm candidate or accepted erratum.','No fullAPI dependency graph or downstream deployment exploitability analysis; input exposure distinctions are explicit.'],
'options':options,'recommended_next_step':'Choose one bounded actual repair-design/implementation boundary around complete equal-hash outcomes, with partial-issuer correctness and compatibility assessed before code is admitted. Retain the baseline, exact counterexample, all success properties and existing budgets. Root owns prior-art/upstream reporting and independent actual Opus review; no owner-signoff/quarantine workaround is proposed.',
'resources':{'build':json.loads((E/'commands.json').read_text())[-1],'cases':json.loads((E/'run-finish.json').read_text()),'free_bytes':shutil.disk_usage(E).free},
'holds':'6095review:changes and5357review:needs-user preserved; all prior evidence and production gates unchanged.'}
(E/'report.json').write_text(json.dumps(report,indent=2)+'\n')
# Public/portable reviewer input: no raw host paths or environment/build logs.
packet='''# Issue6475: completed baseline and production seam evidence

OpenAI GPT-6 Astra, actual xhigh lane. Production remains clean at e53464c73f31f7aca800f3867ac054c36408e346. This supplements the previously reviewed trace/reference evidence; no patch exists or is approved.

## Completed scope

The unmodified Rust12-case matrix is order-invariant for this particular input but label-dependent: each labeling has one canonical output across all6orders, and the two labels give different outputs. Both inputs succeed at the Rust default4000 global-call budget and exactly at4; limits0..3 return HndqCallLimitExceeded. JS first-appearance behavior is not contradicted by a pure renaming: a bijection with quadorder fixed preserves that abstract order.

All86 packaged W3C entries were executed:64canonical-document,21issuer-map,1negative. Rust defaults pass86; JS5 diagnostic4000-call/1s pass86; JS default workfactor1 with1s wall cap passes68 and rejects18positive fixtures. No skips, unknown types, parse errors or external timeouts occurred. The full baseline is from the pinned package manifest; it is not a fresh-current W3C download, a full Rust unit-test execution or a full Sparq workspace run. Exact expected/output bytes and per-case raw command results remain in the hashed bundle.

'''
def block(label,text,lang='text'):
 return f'## {label}\n\n```{lang}\n{text.rstrip()}\n```\n\n'
packet+=block('Sparq seam and CI connection',json.dumps(report['B1'],indent=2),'json')
packet+=block('Budget and secondary issuer-order lead',json.dumps({'B5':report['B5'],'B6':report['B6'],'N2':report['N2']},indent=2),'json')
packet+=block('Actual canonical-byte consumers',json.dumps(consumers,indent=2),'json')
packet+=block('Real repair options and compatibility boundaries',json.dumps(options,indent=2),'json')
packet+='No error-only proposal can make the current success property pass; no property weakening is proposed. The output-selecting option requires real algorithm review and byte-level compatibility evidence, not a claim that the standard already prescribes that tie breaker. Do not treat source-only B6 as a reproduced bug or let it substitute for this task.\n\n'
for f in ['matrix-summary.json','rust-budget-cases.json','suite-counts.json','suite-summary.json','protocol.json','harness/Cargo.toml','harness/src/main.rs','js-case.cjs','ci/counterexample.json','ci/seed.txt']:
 packet+=block(f,(E/f).read_text())
# Embed complete affected seam functions and exact consumer excerpts; full source files retained in manifest.
sections=[('crates/sparq-canon/src/lib.rs',259,278),('crates/sparq-canon/src/lib.rs',304,413),('crates/sparq-canon/src/lib.rs',424,455),('crates/sparq-canon/src/lib.rs',495,510),('crates/sparq-canon/tests/proptest_canon_determinism.rs',140,169),('crates/sparq-canon/tests/proptest_canon_determinism.rs',269,285),('crates/sparq-canon/tests/proptest_canon_determinism.rs',361,401),('crates/sparq-canon/src/concept.rs',271,280),('crates/sparq-wasm/src/canon.rs',21,33),('js/src/dataset.ts',266,284),('js/src/dataset.ts',386,399),('crates/sparq-vc/src/suite.rs',305,321),('crates/sparq-zk/src/commit.rs',238,258),('crates/sparq-zk/src/trace.rs',82,108),('crates/sparq-zk/src/vc_bridge.rs',318,341)]
for f,a,b in sections:
 lines=(S/f).read_text().splitlines();packet+=block(f+f':{a}–{b}','\n'.join(f'{n}: {lines[n-1]}' for n in range(a,b+1)))
for f in ['source/upstream-counter.rs','source/upstream-error.rs']:
 packet+=block(f,(E/f).read_text())
for f,a,b in [('source/upstream-api.rs',132,140),('source/upstream-api.rs',283,292),('source/upstream-canon.rs',112,178),('source/upstream-canon.rs',431,446)]:
 lines=(E/f).read_text().splitlines();packet+=block(f+f':{a}–{b}','\n'.join(f'{n}: {lines[n-1]}' for n in range(a,b+1)))
proof=json.loads((E/'dependency-proof.json').read_text());packet+=block('Exact unmodified archive proof and locked package versions',json.dumps({'archives':proof['archives'],'packages':[p for p in proof['resolved_packages'] if p['name'] in ['rdf-canon','oxrdf','oxttl','sha2','digest']]},indent=2),'json')
packet+='## Retained context and limits\n\nComplete raw expected/output rows, full source files, Cargo.lock, fixture corpus, executable hashes, native toolchain, commands, host resource records and prior-evidence verification are retained locally. This portable packet omits host paths and large full-suite output bodies. The independent review should evaluate the concrete repair boundary after this evidence; no model review is claimed for any proposed algorithm.\n'
assert '/Users/' not in packet and '/private/' not in packet
(E/'public-review-packet.md').write_text(packet)
prior=[]
for name in ['replay','trace','reference-v5']:
 p=E.parent/name;m=json.loads((p/'manifest.json').read_text())
 for item in m['files']:assert H(p/item['path'])==item['sha256'],(name,item['path'])
 prior.append({'name':name,'manifest_sha256':H(p/'manifest.json'),'files':len(m['files']),'verified':True})
(E/'prior-evidence-verification.json').write_text(json.dumps(prior,indent=2)+'\n')
(E/'README.md').write_text('''# Issue6475 evidence completion

[GPT-6 Astra] Production source is unchanged. report.json records scope and conclusions; public-review-packet.md is the portable reviewer input.

The standalone Rust harness uses exact cached registry versions and an APFS clone of this task's own warm replay target. No dependency source was modified. build.py resolves offline then builds locked/offline with twojobs, a300s cap and6GiB free floor. run-cases.py executes the declared matrix/budgets and every entry in fixtures/manifest.jsonld through unmodified Rust and JS5 modules, without retries. See protocol.json for fixed limits; raw stdout, expected output and errors are in commands-cases.json/suite-results.json. Exit0 of a collection harness is not an invariance verdict: matrix-summary.json explicitly records label dependence.

Only the86-entry packaged RDFC10 manifest was scored; legacy URDNA/support files present in the package were not independently treated as extra tests. The Rust upstream test module, Sparq property binary, workspace/engine and broader random campaign were not built or rerun. No source fix, gate change, remote action or model call occurred. Earlier three bundles were rehashed unchanged. Clone cache target/ is regenerable and excluded; binary/canon-baseline is the actual exported executable.

Reproduction writes logs: copy this evidence directory first. Python3 build.py then Python3 run-cases.py reproduces the logged commands under the recorded local toolchain/runtime paths; portable Rust harness and Node case-driver source are included.
''')
files=[{'path':str(p.relative_to(E)),'bytes':p.stat().st_size,'sha256':H(p)} for p in sorted(E.rglob('*')) if p.is_file() and 'target' not in p.relative_to(E).parts and p.name not in ['manifest.json','freeze-verification.json']]
(E/'manifest.json').write_text(json.dumps({'files':files,'excluded':['target/ (regenerable copied task-private build cache)','manifest.json (self)','freeze-verification.json (manifest attestation)']},indent=2)+'\n')
v={'files':len(files),'packet_bytes':(E/'public-review-packet.md').stat().st_size,'packet_sha256':H(E/'public-review-packet.md'),'manifest_sha256':H(E/'manifest.json'),'binary_sha256':H(E/'binary/canon-baseline'),'free_bytes':shutil.disk_usage(E).free,'phase_elapsed_seconds':time.time()-json.loads((E/'start.json').read_text())['epoch']}
(E/'freeze-verification.json').write_text(json.dumps(v,indent=2)+'\n');print(json.dumps(v,indent=2))
