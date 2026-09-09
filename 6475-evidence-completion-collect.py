# [GPT-6 Astra] Collect source-bound seam and consumer evidence; no source edits.
from pathlib import Path
import shutil,json,hashlib,subprocess,tarfile
E=Path(__file__).resolve().parent;W=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue6475');Q=E.parent.parent/'direct-6095/queue-incident'
H=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
paths=['crates/sparq-canon/src/lib.rs','crates/sparq-canon/src/concept.rs','crates/sparq-canon/src/rdf12.rs','crates/sparq-canon/tests/proptest_canon_determinism.rs','crates/sparq-canon/Cargo.toml','crates/sparq-wasm/src/canon.rs','crates/sparq-zk/src/canon.rs','crates/sparq-zk/src/commit.rs','crates/sparq-zk/src/dual_leaf.rs','crates/sparq-zk/src/trace.rs','crates/sparq-zk/src/vc_bridge.rs','crates/sparq-vc/src/suite.rs','crates/sparq-difftest/src/iso.rs','crates/sparq-difftest/Cargo.toml','crates/sparq-bench/src/update_fuzz.rs','js/src/dataset.ts','js/src/wasm.ts','Cargo.lock','Cargo.toml','rust-toolchain.toml']
for f in paths:
 out=E/'source/sparq'/f;out.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(W/f,out)
ci_files={}
for f in ['crates/sparq-canon/src/lib.rs','crates/sparq-canon/tests/proptest_canon_determinism.rs','crates/sparq-canon/Cargo.toml']:
 ci_files[f]={'current_sha256':H(W/f),'failing_ci_source_sha256':H(Q/'source'/f),'identical':(W/f).read_bytes()==(Q/'source'/f).read_bytes()};assert ci_files[f]['identical']
(E/'ci').mkdir()
for f in ['counterexample.json','seed.txt','original.nq','renamed.nq','report.json','vectors-job102661090898-excerpts.txt']:
 shutil.copy2(Q/f,E/'ci'/f)
(E/'ci-source-identity.json').write_text(json.dumps(ci_files,indent=2)+'\n')
U=Path('/Users/jesght/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f');C=Path('/Users/jesght/.cargo/registry/cache/index.crates.io-1949cf8c6b5b557f');archives=[]
for name in ['rdf-canon-0.15.3','oxrdf-0.2.4','oxttl-0.1.8']:
 p=C/(name+'.crate');checked=0
 with tarfile.open(p) as t:
  for x in t.getmembers():
   if x.isfile():
    rel=Path(x.name).relative_to(name);assert (U/name/rel).read_bytes()==t.extractfile(x).read(),rel;checked+=1
 archives.append({'package':name,'crate_sha256':H(p),'source_files_verified':checked,'all_match':True})
meta=json.loads((E/'resolve.json').read_text());featuremap={n['id']:n['features'] for n in meta['resolve']['nodes']}
(E/'dependency-proof.json').write_text(json.dumps({'archives':archives,'resolved_packages':[{'name':p['name'],'version':p['version'],'source':p['source'],'features':featuremap[p['id']]} for p in meta['packages']]},indent=2)+'\n')
(E/'binary').mkdir();shutil.copy2(E/'target/debug/canon-relabel-replay',E/'binary/canon-baseline')
state={'head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=W,text=True).strip(),'status':subprocess.check_output(['git','status','--porcelain=v1'],cwd=W,text=True)};assert not state['status'];(E/'source-state.json').write_text(json.dumps(state,indent=2)+'\n')
print('source copied; CI identity and archive members verified; source clean')
