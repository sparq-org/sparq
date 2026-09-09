# [GPT-6 Astra] Freeze local evidence and a public-source-only review packet.
from pathlib import Path
import json,hashlib,shutil,time
E=Path(__file__).resolve().parent
H=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
links={
'conformance':'https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#conformance',
'canonicalization':'https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#canon-algorithm',
'related':'https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#hash-related-algorithm',
'ndegree':'https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#hash-nd-quads-algorithm',
'upstream_rust':'https://github.com/zkp-ld/rdf-canon/tree/v0.15.3',
'reference':'https://github.com/digitalbazaar/rdf-canonize/tree/7d06569ac53441bc4786943d2336c98cbb29d1bb',
'reference_metadata':'https://registry.npmjs.org/rdf-canonize/4.0.1'}
# Exact anchors are checked against the downloaded dated Recommendation below.
html=(E/'rdfc-20240521.html').read_text()
for name in ['canonicalization','related','ndegree','conformance']:
 anchor=links[name].split('#')[1]
 assert ('id="'+anchor+'"') in html,anchor
report={
'issue':6475,'phase':'bounded scratch trace/localization; no production correction',
'author':'OpenAI GPT-6 Astra; inherited xhigh lane',
'head':'e53464c73f31f7aca800f3867ac054c36408e346',
'decision':'No responsible Rust-only algorithm patch established. Seek soundness review of a provisional shared-algorithm counterexample before selecting a repair.',
'confirmed':[
'Scratch-only instrumentation leaves direct upstream output byte-identical to phase1 and Linux CI.',
'b2 and b3 have the same first-degree hash 166b37005906f602b8c3e82241f325ff2d22e757fe1ec677be8f8684c7805218; anchors b0/b4 are c14n0/c14n1.',
'All four top-level N-degree calls (b2,b3,zz3,zz4) have identical preimage bytes and SHA256 a812f907ae7c594b7fd02586a57c254860f399e64e6b3bbf13fa0e56e38f81e8. This is information loss before hashing, not a cryptographic collision.',
'Related hashes encode s+q+anchor, g+other and o+q+other independently. Grouping discards which anchor shares the quad with each object/graph role.',
'Stably sorting tied hash results preserves BTreeMap input-label order, choosing b2 before b3 and zz3 before zz4. The temporary issuers are swapped; canonical outputs differ.',
'Unmodified Digital Bazaar rdf-canonize4.0.1 exactly preserves relabeling for the provided input order, but reversing only quad order changes canonical output for each dataset.',
'The reference default budget rejects both inputs at2 deep iterations. Reported ordering defect uses explicit maxDeepIterations16 and timeout1000ms only in this isolated diagnostic; no production budget changed.',
'Reference one-edge positive control and existing W3C test021c/test058c/test073c pass. This is not full conformance validation.'
],
'normative_assessment':{
'provisional':'The traced information-loss path agrees with the visible normative steps. A standard-level counterexample is plausible, not an accepted erratum or complete compliance judgment.',
'steps':['4.7.3 steps1-5 encode position/predicate/related identifier, not the full correlated quad.','4.8.3 steps3.1.1-3.1.2 aggregate these independently into Hn.','4.4.3 steps5.2-5.3 sort N-degree results only by hash, then issue labels in each result issuer order.'],
'not_responsible':['The >= versus > pruning discrepancy is not exercised: every Hn group for this pair has one member and a single permutation, with no prior chosen path.','Lexical b10 versus b2 issuer-order risk is not exercised: only b0 and b1 temporary identifiers exist.','No repeated node within a quad or duplicate quad exists; mention-set multiplicity is not the cause.','Sparq bridge, parser, proptest RNG, and default randomized HashMap iteration are unnecessary for the failure.'],
'links':links},
'candidate_correction':None,
'why_no_patch':'Adding quad context would change the specified hash algorithm. Sorting original labels, changing input ordering, or silently rejecting the saved shape does not establish conformant canonicalization. Tied hashes can also represent genuine symmetry; blanket tie rejection is not justified.',
'next_smallest_step':'Have independent Opus review exact trace, topology and primary-source correspondence. If confirmed, root can prepare an upstream/W3C reproducer and choose a compatibility-preserving repair or explicitly reviewed fail-closed contract; do not weaken the invariance property.',
'ownership':'Phase1 focused ownership check remains authoritative: PR5357 documentation-only bridge removal is review:needs-user. Preserve that hold and PR6095 review:changes/live evidence. No fresh remote PR mutation/read required here.',
'limits':['One pinned reference version, not a latest-version survey.','Reference synchronous internal module invoked directly to avoid unrelated async dependency loading; actual unmodified source and package archive retained.','Reference has independent implementation structure but may share standard lineage; not an independent mathematical proof.','No candidate correction, Rust upstream fixture suite, broader randomized campaign, full workspace, or engine build run.','Only three packaged W3C fixtures executed, on JS reference.','No standard-level claim accepted before independent review.'],
'resources':{'rust_build_and_replay_seconds':json.loads((E/'commands.json').read_text())[-1]['elapsed_total_seconds'],'rust_floor_bytes':6*1024**3,'free_at_report':shutil.disk_usage(E).free,'build_jobs':2,'target':'task-owned trace/target','no_installs':True,'no_remote_mutations':True,'no_production_edits':True},
'evidence':['replay.log','replay.stderr.log','nd-summary.json','topology-proof.json','instrumentation.diff','dependency-proof.json','reference-replay.json','reference-fixtures.json','reference-package-metadata.json','source-state.json','commands.json']}
(E/'report.json').write_text(json.dumps(report,indent=2)+'\n')
(E/'download-evidence.json').write_text(json.dumps({'requests':[{'url':links['reference_metadata'],'file':'reference-package-metadata.json','status':'curl exit0'},{'url':'https://registry.npmjs.org/rdf-canonize/-/rdf-canonize-4.0.1.tgz','file':'rdf-canonize-4.0.1.tgz','status':'curl exit0; metadata sha512 integrity matched'},{'url':'https://www.w3.org/TR/2024/REC-rdf-canon-20240521/','file':'rdfc-20240521.html','status':'curl exit0'}],'earlier_web_tool_limit':'raw GitHub v4.0.1 package.json/RDFC10Sync web reads returned cache miss; no package execution based on these failed reads. Public npm archive was used.'},indent=2)+'\n')
packet='''# Issue6475: upstream canonicalization trace for independent soundness review

Author: OpenAI GPT-6 Astra, actual xhigh lane. This is diagnosis, not an independently reviewed repair.
Production source remains exact e53464c73f31f7aca800f3867ac054c36408e346. No production edit or candidate correction.

Please assess whether the loss of quad correlation below is a counterexample to the normative RDFC1.0 algorithm, a Rust-only implementation error, or an incomplete diagnosis. Do not treat the standard-level explanation as established. Recommend the smallest responsible next step while preserving canonical-output compatibility, invariance testing, and bounded computation. No policy or gate weakening is proposed.

## Findings

'''
packet+='\n'.join('- '+s for s in report['confirmed'])+'\n\n'
packet+='The sole p edge fixes b4 and b0, so swapping b2/b3 while retaining these anchors is not an automorphism. Both nodes see the same multiset of independently hashed related positions, despite distinct quad correlations. The tie is therefore not justified merely by the existence of equal hashes.\n\n'
packet+='Normative correspondence (provisional): §4.7.3 steps1–5 build related hashes; §4.8.3 steps3.1.1–3.1.2 place them independently into Hn; §4.4.3 steps5.2–5.3 sort N-degree results by hash and assign issuer order. The selected path appears to follow these steps. The contract expects isomorphic datasets to canonicalize identically.\n\n'
packet+='\n'.join(f'- {k}: {v}' for k,v in links.items())+'\n\n'
packet+='The >= pruning discrepancy cannot explain this input: all related groups are singletons and no prior chosen path exists. Temporary IDs never exceed b1. Dataset multiplicity and bridge parsing are also absent. These observations rule out the tempting small fixes, not every possible implementation defect.\n\n'
packet+='## Limits and next step\n\n'+'\n'.join('- '+s for s in report['limits'])+'\n\n'+report['why_no_patch']+'\n\n'+report['next_smallest_step']+'\n\n'
packet+='The independent reference package is rdf-canonize4.0.1, npm gitHead7d06569ac53441bc4786943d2336c98cbb29d1bb; archive integrity matched the registry SHA512. Its RDFC10Sync, NQuads, IdentifierIssuer, Permuter and built-in Node crypto code are unchanged. Direct synchronous invocation is the same algorithm selected by its _canonizeSync RDFC1.0 branch, without loading async setimmediate. Reference default rejection and bounded diagnostic outcomes are deliberately separate.\n\n'
def block(label,text,lang='text'):
 return f'## {label}\n\n```{lang}\n{text.rstrip()}\n```\n\n'
for f in ['harness/src/main.rs','instrumentation.diff','replay.log','reference-replay.cjs','reference-replay.json','reference-fixtures.cjs','reference-fixtures.json','topology-proof.json']:
 packet+=block(f,(E/f).read_text())
packet+=block('Relevant trace: H1, Hn, N-degree preimages and canonical issuance','\n'.join(x for x in (E/'replay.stderr.log').read_text().splitlines() if x.startswith(('TRACE h1','TRACE hn','TRACE nd','TRACE issue prefix=c14n'))))
# Source-only excerpts: original pinned code, before instrumentation, with exact original line numbers.
source=(E/'upstream-traced/src/canon.rs').read_text()
original=Path('/Users/jesght/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rdf-canon-0.15.3/src/canon.rs').read_text().splitlines()
for a,b,label in [(254,310,'Rust first-degree grouping and unique anchors'),(327,449,'Rust hash-path construction, stable sort and issuer consumption'),(602,685,'Rust related hash and result ordering'),(707,1140,'Rust complete N-degree processing body')]:
 packet+=block(label+' — rdf-canon0.15.3 src/canon.rs', '\n'.join(f'{i}: {original[i-1]}' for i in range(a,b+1)))
js=(E/'reference/package/lib/RDFC10Sync.js').read_text().splitlines()
for a,b,label in [(127,165,'Reference hash-path sort and issuer consumer'),(239,270,'Reference related hash'),(436,522,'Reference grouping and related helper')]:
 packet+=block(label+' — rdf-canonize4.0.1 lib/RDFC10Sync.js','\n'.join(f'{i}: {js[i-1]}' for i in range(a,min(b,len(js))+1)))
packet+='## Packet boundary\n\nThis packet intentionally omits host paths, environment, compiler/build logs and private operational metadata. Those remain in local hashed evidence. Complete unmodified reference sources, original Rust package/archive provenance, full trace and exact executable are retained locally; only the listed load-bearing source ranges are embedded here. No fresh survey of W3C errata or latest reference versions is claimed.\n'
assert '/Users/' not in packet and '/private/' not in packet and '/opt/homebrew/' not in packet
(E/'public-review-packet.md').write_text(packet)
(E/'README.md').write_text('''# Issue6475 phase2 evidence

[GPT-6 Astra] No production changes. `report.json` records confirmed findings and limits. `public-review-packet.md` excludes host/private execution details and is the intended independent review input.

Reproduction: `python3 run.py` recreates the instrumented offline Rust diagnostic in this directory and expects the saved pair to fail with exit1. `node reference-replay.cjs` uses the unmodified extracted pinned reference package; `node reference-fixtures.cjs` runs the three fixture controls. Use the frozen command JSON for the exact installed executables and environment actually used. Re-execution would overwrite diagnostic log paths: copy this bundle first.

The only upstream source-copy mutation is the instrumentation diff; no algorithm correction was attempted. Package SHA512 verifies the reference download, and phase1 package provenance verifies the Rust source. Do not interpret the JS default complexity error as successful canonicalization, or the diagnostic explicit iteration budget as a production policy change.

Private target is regenerable and excluded from the manifest; the exact traced executable is exported under binary/. Parent phase1 evidence remains immutable.
''')
files=[]
for p in sorted(E.rglob('*')):
 if p.is_file() and 'target' not in p.relative_to(E).parts and p.name not in ('manifest.json','freeze-verification.json'):
  files.append({'path':str(p.relative_to(E)),'bytes':p.stat().st_size,'sha256':H(p)})
(E/'manifest.json').write_text(json.dumps({'files':files,'exclusions':['target/ (regenerable private compilation output)','manifest.json (self)','freeze-verification.json (manifest attestation)']},indent=2)+'\n')
verification={'files':len(files),'packet_bytes':(E/'public-review-packet.md').stat().st_size,'packet_sha256':H(E/'public-review-packet.md'),'manifest_sha256':H(E/'manifest.json'),'binary_sha256':H(E/'binary/canon-relabel-traced'),'free_bytes':shutil.disk_usage(E).free,'phase2_scratch_elapsed_seconds':time.time()-json.loads((E/'started.json').read_text())['started_epoch']}
(E/'freeze-verification.json').write_text(json.dumps(verification,indent=2)+'\n')
print(json.dumps(verification,indent=2))
