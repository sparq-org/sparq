# [GPT-6 Astra] Freeze exact diagnostic evidence; no production changes.
from pathlib import Path
import hashlib,json,subprocess,shutil,datetime,tomllib
E=Path(__file__).resolve().parent; W=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue6475')
base='e53464c73f31f7aca800f3867ac054c36408e346'; sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=W,text=True).strip()==base
assert not subprocess.check_output(['git','status','--porcelain'],cwd=W)
commands=json.loads((E/'commands.json').read_text());assert [r['exit'] for r in commands]==[0,0,0,0,1]
fixture=json.loads((E/'counterexample.json').read_text());lines=(E/'replay.log').read_text().splitlines()
left=json.loads(next(l.partition('=')[2] for l in lines if l.startswith('original_canonical=')));right=json.loads(next(l.partition('=')[2] for l in lines if l.startswith('renamed_canonical=')))
assert left==fixture['logged_left'] and right==fixture['logged_right'] and left!=right
for name,expected in [('original',fixture['original_nquads']),('renamed',fixture['renamed_nquads'])]:
 actual=''.join(l.removeprefix('input_'+name+': ')+'\n' for l in lines if l.startswith('input_'+name+': '))
 assert actual==expected
(E/'binary').mkdir(exist_ok=True);shutil.copyfile(E/'target/debug/canon-relabel-replay',E/'binary/canon-relabel-replay')
for f in ['crates/sparq-canon/src/lib.rs','crates/sparq-canon/Cargo.toml','crates/sparq-canon/tests/proptest_canon_determinism.rs']:
 p=E/'sparq-source'/f;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(subprocess.check_output(['git','show',base+':'+f],cwd=W))
lock=tomllib.loads((W/'Cargo.lock').read_text());hlock=tomllib.loads((E/'harness/Cargo.lock').read_text());mainkeys={(p['name'],p['version'],p.get('checksum')) for p in lock['package']}
extra=[(p['name'],p['version'],p.get('checksum')) for p in hlock['package'] if p.get('source') and (p['name'],p['version'],p.get('checksum')) not in mainkeys]
(E/'lock-comparison.json').write_text(json.dumps({'registry_packages_not_identical_to_main_lock':extra},indent=2)+'\n')
source_urls={'spec':'https://www.w3.org/TR/2024/REC-rdf-canon-20240521/','related_hash':'https://www.w3.org/TR/rdf-canon/#hash-related-blank-node','n_degree':'https://www.w3.org/TR/rdf-canon/#hash-nd-quads','errata':'https://w3c.github.io/rdf-canon/errata/','upstream':'https://github.com/zkp-ld/rdf-canon'}
(E/'primary-sources.json').write_text(json.dumps({'urls':source_urls,'read_via':'Web primary-source read; cached tagged crate sources independently verified against registry archive checksum.','source_inference':'The specification requires isomorphic datasets to canonicalize identically. Its related-node algorithm combines related position, predicate unless graph position, then related-node identifier/hash. Changing that construction needs interoperability review; no spec-conformance repair is claimed here.','errata_limit':'Rendered errata page contains workflow and empty issue lists; that does not establish absence of relevant errata.'},indent=2)+'\n')
# Source-derived localization, distinct from direct executable evidence.
report={
 'issue':6475,'base':base,'worktree':str(W),'branch':'codex/canon-relabel-invariance','worktree_clean':True,
 'provenance':'Actual OpenAI GPT-6 Astra xhigh authored the scratch harness and evidence. Production source and cached dependencies were not modified.',
 'result':'REPRODUCED_AND_LOCALIZED_UPSTREAM. The direct rdf-canon0.15.3/oxrdf0.2.4 harness emits byte-identical left/right canonical strings to the Linux CI failure and exits1; the one-edge positive control passes. No Sparq crate, bridge, parser, proptest or engine is linked.',
 'executed':{'build':'cargo build --locked --offline','runtime':'native aarch64-apple-darwin, pinned rustc1.97.1 (8bab26f4f68e0e26f0bb7960be334d5b520ea452)','tests':'One exact saved three-quad pair plus one-edge calibration in a standalone main, not cargo test. No randomized cases or performance measurements.','elapsed_total_seconds':commands[-1]['elapsed_total_seconds'],'jobs':2,'free_before_bytes':commands[0]['free_before'],'free_after_bytes':commands[-1]['free_after'],'floor_bytes':6*1024**3,'limit_seconds':300,'limits_triggered':False,'registry_package_drift_from_main':extra},
 'dependency_proof':'Two direct dependencies,40 resolved packages. rdf-canon has no enabled features; oxrdf default. Locked checksum matches each cached .crate archive; all311 rdf-canon and16 oxrdf archive files equal expanded source. Full metadata/lock/feature and binary/source hashes preserved.',
 'bridge_exclusion':'The input quads are constructed directly as oxrdf0.2.4 values with checked term constructors. Their printed N-Quads exactly equal the saved source-derived CI fixtures. No oxrdf0.3→text→oxrdf0.2 bridge runs. This independently reproduces the failure below the bridge, so changing Sparq serialization is not the repair boundary.',
 'mechanism_evidence':'Sparq canonicalize_quads atlib.rs310–312 simply bridges and invokes rdf_canon. Upstream canon.rs602–662 constructs per-related-node hashes;708–854 groups subject/object/graph mentions;320–449 uses N-degree hashes and issuer order. The two ambiguous nodes b2/b3 exchange object versus graph-name positions across the q quads; fixed anchors b4/b0 are distinguished by the p triple. Their first-degree descriptions coincide; the related-node representation is the narrow investigation target. The replay proves an upstream path failure, but this phase did not instrument internal hashes or prove which algorithm line must change.',
 'repair_recommendation':{'boundary':'Persist the exact pair/seed as a permanent Sparq regression, then repair or patch-pin the upstream canonicalizer component shared by all standard canonicalize/issue/digest consumers. Keep the bridge, proptest case count and gate policy unchanged.','not_yet_justified':'No safe one-line production correction is established. Sorting original labels, swallowing failure, special-casing these quads, or retrying for green would mask the defect. A speculative change to related-hash inputs may alter standardized output and needs specification/upstream comparison plus W3C regression validation.','next_bounded_step':'Use the frozen minimal harness to capture upstream N-degree hash/issuer trace on this pair and compare the same pair with an independent established implementation before choosing a narrow dependency patch. Preserve exact baseline outputs and all existing conformance/poison-input limits.'},
 'ownership':{'area_query':'repo:sparq-org/sparq is:pr is:open label:area:sparq-canon','area_results':0,'mention_query':'repo:sparq-org/sparq is:pr is:open "sparq-canon"','mention_results':[5357,6148],'relevant_pr':{'number':5357,'head':'d460484e050889b931cc4b83b5cae09a09992d6f','state':'OPEN DRAFT','labels':['review:needs-user','area:docs'],'files':['research/sparq-canon-oxrdf03-bridge-removal.md'],'action':'Preserve its hold and historical authorship; this diagnostic does not adopt its bridge-removal design or supersede its review. Its only source change is documentation, so no confirmed Rust hunk conflict.'},'limits':'Focused searches and5357 file listing, not a complete path census of every open PR.6148 title/body indicates site/guide documentation; its files were not fetched.'},
 'upstream_lookup':{'query':'repo:zkp-ld/rdf-canon is:issue relabel','count':0,'limits':'One focused lookup does not establish no upstream duplicate or fix.'},
 'scope_limits':['Native macOS reproduction, not new Linux/wasm conformance.','No actual Sparq library build due disk/engine dev-dependency constraints; original CI supplies the actual Sparq-path failure.','No production repair, commit, push, rerun, requeue, model invocation, registry/release/EC2 change.','All6095 evidence remains unchanged; issue6475 worktree stays clean.'],
 'paths':{'target':str(E/'target'),'target_allocated_kib':int(subprocess.check_output(['du','-sk',str(E/'target')],text=True).split()[0]),'binary':str(E/'binary/canon-relabel-replay'),'harness':str(E/'harness'),'input_bundle_manifest':'1ef65eb20334c6934a82876cf712658c7bb383ac4f344107e4757da45ba46239'}
}
(E/'report.json').write_text(json.dumps(report,indent=2)+'\n')
readme='''# Issue6475 deterministic replay

[GPT-6 Astra] This standalone diagnostic constructs the saved three-quad pair directly in oxrdf0.2.4 and invokes pinned rdf-canon0.15.3, with no Sparq bridge or engine. Exit1 is the reproduced failure; the calibration exits normally before the failing pair. It is not a passing test suite or a proposed patch.

Run `python3 run.py` from this evidence directory. It resolves only cached dependencies, formats the exact harness, builds offline/locked with the installed pinned compiler and two jobs, then executes the diagnostic. The runner stops at300seconds or below6GiB free disk. To inspect existing evidence without rebuilding, read commands.json/replay.log/replay.stderr.log. The exported binary can be run directly on the recorded native host architecture.

The final manifest excludes the regenerable private target directory; its path and size are recorded. It includes the exported binary, final formatted source, Cargo.lock, complete resolved dependency graph, actual output and upstream/source provenance. Existing cached registry sources are verified against their exact .crate archive checksums, not edited.

Production repair remains unproven. A change to the shared canonicalizer must preserve conformance, default outputs, hash profiles and poison-input budgets; do not modify the bridge or retry the random property until it passes. See report.json for the precise next bounded experiment and ownership limitations.
'''
(E/'README.md').write_text(readme)
packet='# Issue6475 replay/localization\n\n```json\n'+json.dumps(report,indent=2)+'\n```\n\n## Exact harness\n\n```rust\n'+(E/'harness/src/main.rs').read_text()+'\n```\n\n## Actual output\n\n```text\n'+(E/'replay.log').read_text()+(E/'replay.stderr.log').read_text()+'\n```\n\n## Commands\n\n```json\n'+(E/'commands.json').read_text()+'\n```\n'
(E/'review-packet.md').write_text(packet)
rows=[dict(path=str(p.relative_to(E)),bytes=p.stat().st_size,sha256=sha(p)) for p in sorted(E.rglob('*')) if p.is_file() and 'target' not in p.relative_to(E).parts and p.name not in ('manifest.json','freeze-verification.json')]
(E/'manifest.json').write_text(json.dumps({'created_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'base':base,'files':rows},indent=2)+'\n')
for row in rows:assert sha(E/row['path'])==row['sha256']
verification={'files':len(rows),'packet_bytes':(E/'review-packet.md').stat().st_size,'packet_sha256':sha(E/'review-packet.md'),'manifest_sha256':sha(E/'manifest.json'),'binary_sha256':sha(E/'binary/canon-relabel-replay'),'free_disk_bytes':shutil.disk_usage(E).free,'clean':True}
(E/'freeze-verification.json').write_text(json.dumps(verification,indent=2)+'\n');print(json.dumps(verification,indent=2))
