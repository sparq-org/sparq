# [GPT-6 Astra] Freeze compatibility evidence without changing earlier bundles.
from pathlib import Path
import json,hashlib,base64,shutil,time,difflib,collections,subprocess
E=Path(__file__).resolve().parent
H=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
d=json.loads((E/'reference-replay.json').read_text());m=json.loads((E/'package-metadata.json').read_text());fixtures=json.loads((E/'reference-fixtures.json').read_text())
assert d['summary']==dict(default_errors=12,diagnostic_successes=12,distinct_outputs=2,same_order_relabel_equal=True,order_invariant=False)
assert all(x['defaultResult'].get('error')=='Maximum deep iterations exceeded (2).' for x in d['all_orders'])
assert d['positive_control_equal'] and fixtures['passed']==3
for f in ['original.nq','renamed.nq']:assert (E/f).read_bytes()==(E.parent/'replay'/f).read_bytes()
old=json.loads((E.parent/'trace/reference-replay.json').read_text())
for name in ['exact_pair','sorted_lines','reverse_lines']:assert d[name]==old[name]
preserved=[]
for name in ['replay','trace']:
 p=E.parent/name;manifest=json.loads((p/'manifest.json').read_text());files=manifest['files']
 for x in files:
  # Both earlier immutable manifests use relative paths.
  assert H(p/x['path'])==x['sha256'],(name,x['path'])
 preserved.append(dict(bundle=name,files=len(files),manifest_sha256=H(p/'manifest.json'),unchanged=True))
(E/'previous-evidence-verification.json').write_text(json.dumps(preserved,indent=2)+'\n')
ref4=E.parent/'trace/reference/package/lib/index.js';ref5=E/'reference/package/lib/index.js'
(E/'wrapper-4-to-5.diff').write_text(''.join(difflib.unified_diff(ref4.read_text().splitlines(True),ref5.read_text().splitlines(True),fromfile='rdf-canonize4.0.1/lib/index.js',tofile='rdf-canonize5.0.0/lib/index.js')))
outputs={}
for row in d['all_orders']:
 h=hashlib.sha256(row['output'].encode()).hexdigest();outputs.setdefault(h,{'output':row['output'],'cases':[]})['cases'].append({'labeling':row['labeling'],'order':row['order']})
public={
'author':'OpenAI GPT-6 Astra; actual xhigh lane',
'package':'rdf-canonize5.0.0','gitHead':m['gitHead'],'metadata_url':'https://registry.npmjs.org/rdf-canonize/5.0.0','archive_url':m['dist']['tarball'],'archive_sha256':H(E/'rdf-canonize-5.0.0.tgz'),'archive_integrity':m['dist']['integrity'],'integrity_verified':True,
'node':d['node'],'algorithm':'unmodified RDFC10Sync directly; no async loader, install, native addon or lifecycle script',
'diagnostic_options':d['options'],'defaults':'No options passed; maxWorkFactor1 calculates2 deep iterations for the saved pair. All12 orders rejected with the same documented error.',
'summary':d['summary'],'single_edge_positive_control':d['positive_control_equal'],'existing_W3C_fixture_controls':{'passed':fixtures['passed'],'ids':[x['id'] for x in fixtures['results']]},
'outputs':outputs,
'exact_phase2_cases_byte_identical':True,
'algorithm_source_comparison':'RDFC10Sync.js, RDFC10.js, IdentifierIssuer.js, MessageDigest.js, NQuads.js and Permuter.js are each byte-identical to4.0.1. Complete per-file hashes retained.',
'conclusion':'Current5.0.0 reference does not remove the observed order dependence under the fixed bounded diagnostic options. This corroborates phase2 behavior; standard-level soundness interpretation remains provisional for independent review.',
'limits':['Default rejection is not canonicalization success.','Same-order relabeling agrees; the independently observed defect here is quad-order dependence.','Only the declared counterexample/orders and three W3C controls were executed, not full conformance or proof.','Direct synchronous module invocation was inspected and uses built-in Node crypto; normal async public API was not executed.','No production edit, dependency change, gate weakening, remote mutation, installation or new model call.']}
report=dict(public)
report['resources']={'commands':json.loads((E/'commands.json').read_text()),'free_at_freeze':shutil.disk_usage(E).free,'floor_bytes':6*1024**3,'elapsed_seconds':time.time()-json.loads((E/'start.json').read_text())['epoch']}
(E/'report.json').write_text(json.dumps(report,indent=2)+'\n')
packet='''# Issue6475: current rdf-canonize5.0.0 reference supplement

OpenAI GPT-6 Astra, actual xhigh lane. Complements the frozen phase2 public packet SHA256 1f862c034d10297101c8c334a8c70866c1ce1dc99143063a86b04adde233259d; it does not revise that packet or assert a standard-level verdict.

The package version and gitHead match tag9502eee92d87e8685d96b9d325477d767c86fb83. The unmodified archive matches npm SHA512 integrity and SHA1. Source is extracted task-locally without running install/lifecycle scripts. Node24.19 executes the same direct synchronous module seam as phase2. The explicit diagnostic options remain maxDeepIterations16 and timeout1000ms; defaults are recorded separately without tuning.

All6 quad orders across both original and renamed labels return2 distinct canonical strings under those diagnostic options. Same-order relabeling agrees. All12 default cases instead reject at2 deep iterations. The one-edge control and packaged W3C test021c,058c,073c pass. The tested algorithm/helper source files are byte-identical to4.0.1. Thus5.0.0 does not fix the observed ordering defect; the normative interpretation remains for independent review.

'''
def block(name,content,language='text'):
 return f'## {name}\n\n```{language}\n{content.rstrip()}\n```\n\n'
packet+=block('Identity, conclusions, complete output association and limits',json.dumps(public,indent=2),'json')
for f in ['reference-replay.cjs','reference-replay.json','reference-fixtures.cjs','reference-fixtures.json','source-comparison.json','wrapper-4-to-5.diff']:
 packet+=block(f,(E/f).read_text())
packet+='This source-only supplement omits host paths, environment and operational/build logs. Full unmodified package source, archive, commands and integrity evidence remain in its local hashed bundle. No W3C or upstream issue status is claimed here.\n'
assert '/Users/' not in packet and '/private/' not in packet
(E/'public-review-supplement.md').write_text(packet)
(E/'README.md').write_text('''# Current-reference compatibility check

[GPT-6 Astra] Run `node reference-replay.cjs` and `node reference-fixtures.cjs` from a copy of this directory. The unmodified pinned package lives under reference/package. Exact runtime/argv and observed free disk are in commands.json. `run.py` binds the original local execution and checks a6GiB floor. No install, lifecycle script, async loader or global dependency is used.

The process returns0 because all declared cases were collected; inspect order_invariant=false rather than treating exit0 as a soundness pass. Default-budget errors are separately recorded from bounded diagnostic successes. Inputs,12 permutation records and complete canonical outputs are included. Earlier frozen bundles were hash-verified unchanged. No production source was edited.
''')
files=[dict(path=str(p.relative_to(E)),bytes=p.stat().st_size,sha256=H(p)) for p in sorted(E.rglob('*')) if p.is_file() and p.name not in ('manifest.json','freeze-verification.json')]
(E/'manifest.json').write_text(json.dumps({'files':files},indent=2)+'\n')
v=dict(files=len(files),packet_bytes=(E/'public-review-supplement.md').stat().st_size,packet_sha256=H(E/'public-review-supplement.md'),manifest_sha256=H(E/'manifest.json'),free_bytes=shutil.disk_usage(E).free)
(E/'freeze-verification.json').write_text(json.dumps(v,indent=2)+'\n');print(json.dumps(v,indent=2))
