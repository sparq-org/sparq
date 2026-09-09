# [GPT-6 Astra] Read-only classification proof from captured payloads, no live I/O.
from pathlib import Path
import sys, importlib.util, json, subprocess
sys.dont_write_bytecode=True
root=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/issue4246')
p=Path(__file__).parent
calls=[]
def poison(*a,**k):
 calls.append(str(a));raise AssertionError('no subprocess/network allowed')
subprocess.run=poison
subprocess.Popen=poison
def load(name,path):
 spec=importlib.util.spec_from_file_location(name,root/path)
 module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module
ta=load('triage_area','scripts/triage-area.py')
ta._gh=poison;ta.apply_row=poison
ready=load('readiness','scripts/ready-issues.py')
crates=ta.crate_names();roots=ready.workspace_roots(str(root))
rows=[]
for n in [5016,6468,2658]:
 issue=json.loads((p/f'issue{n}.json').read_text())
 classified=ta.classify(issue['title'],issue['body'],crates)
 plan=ta.plan([issue],crates)[0]
 rows.append(dict(number=n,title=issue['title'],labels=sorted(ta.label_names(issue)),raw_classify=classified,planned_areas=plan[1],plan_evidence=plan[2]))
assert rows[0]['planned_areas']==['area:sparq-wrapper-gen']
assert rows[1]['planned_areas']==['area:sparq-wrapper-gen']
assert rows[2]['planned_areas']==[] # existing author label wins; no mutation
aliases={k:ready.partition_path(k,roots) for k in ['wrapper','sparq-wrapper','sparq-wrapper-gen']}
conflicts={k:ready.keys_conflict(k,'sparq-wrapper-gen',roots) for k in ['wrapper','sparq-wrapper']}
assert not any(conflicts.values())
unknown=sorted(set(rows[0]['planned_areas']+rows[1]['planned_areas'])-{'area:ci','area:wrapper','area:sparq-wrapper'})
assert unknown==['area:sparq-wrapper-gen']
assert calls==[]
print(json.dumps(dict(source_ref='a42a9e89dec485f6a319c47cb3635c59cb5a2270',rows=rows,partition_paths=aliases,proposed_aliases_conflict_with_actual_gen=conflicts,unknown_against_fixture_known_labels=unknown,network_or_subprocess_calls=calls,main_not_called=True),indent=2))
