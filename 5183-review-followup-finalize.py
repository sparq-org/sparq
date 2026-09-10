from pathlib import Path
import json,hashlib,difflib,os
p=Path('/private/tmp/sparq-pr6049/.throughput-monitor/direct-5183/review-followup');s=(p/'final.rs.txt').read_text();packet=(p/'review-packet.md').read_text()
a=packet.index('\n### all new and changed semantic tests');b=packet.index('\n### generator integer pool',a)
parts=[]
for name in ['generated_integer_domain_is_injective_including_load','lexical_adjudication_preserves_rows_and_blank_node_structure','injected_marker_fails_during_actual_lexical_adjudication']:
 start=s.index('    fn '+name+'(');end=s.find('\n    #[test]',start)
 assert end>start
 parts.append('    #[test]\n'+s[start:end])
packet=packet[:a]+'\n### Complete changed test functions\n\n```rust\n'+'\n'.join(parts)+'\n```\n'+packet[b:]
a=packet.index('\n### normalized-error-old-detail-control exact scratch delta');b=packet.index('\n## Context boundaries',a)
pos=(p/'controls/normalized-error-fault-positive/update_fuzz.rs').read_text();neg=(p/'controls/normalized-error-old-detail-control/update_fuzz.rs').read_text()
delta=''.join(difflib.unified_diff(pos.splitlines(True),neg.splitlines(True),fromfile='fault-positive/update_fuzz.rs',tofile='fault-negative/update_fuzz.rs'))
(p/'fault-positive-to-negative.diff').write_text(delta)
packet=packet[:a]+'\n### Normalized-error negative: exact delta from the fault-positive source above\n\nThe injected third-call error and appended test are byte-identical; only error-detail handling changes.\n\n```diff\n'+delta+'```\n'+packet[b:]
packet=packet.replace('all new semantic tests','all changed semantic test bodies').replace('old legacy tests are not repeated here','old legacy tests, unchanged historical regression and other unchanged parent tests are not repeated here')
assert '/private/tmp/' not in packet and '/Users/' not in packet
(p/'review-packet.md').write_text(packet)
(p/'packet-scope.json').write_text(json.dumps({'format':'focused follow-up, repository-relative public source','included':'Full delta; complete compare/Verdict/comparable/parse/normalizers/record/one_sided/nquads_line/PROBES/apply_sequence/allowlist; complete three changed test functions; old record helper; Cargo edition inheritance/toolchain configuration; actual suite stdout and structured controls; exact fault-positive source delta plus minimal negative delta.','omitted':'Unchanged operation constructors, LoadSandbox body, engine/parser/canon internals, historical/other unchanged parent tests. Complete current source remains final.rs.txt, prior full source separately frozen. No claim prior review packet was complete.','packet_bytes':len(packet.encode())},indent=2)+'\n')
# Hash all finite evidence, never the regenerable external target; no symlinks allowed.
files=[]
for q in sorted(p.rglob('*')):
 if not q.is_file() or q.name in ['manifest.json','manifest.sha256']:continue
 assert not q.is_symlink(),q
 b=q.read_bytes();files.append({'path':str(q.relative_to(p)),'bytes':len(b),'sha256':hashlib.sha256(b).hexdigest()})
manifest={'head':'fe3284199db0831f353d2ae401b8c69472764904','files':files}
(p/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');digest=hashlib.sha256((p/'manifest.json').read_bytes()).hexdigest();(p/'manifest.sha256').write_text(digest+'  manifest.json\n')
for x in files:assert hashlib.sha256((p/x['path']).read_bytes()).hexdigest()==x['sha256']
print(json.dumps({'head':manifest['head'],'files':len(files),'manifest_sha256':digest,'packet_bytes':len(packet.encode()),'packet_sha256':hashlib.sha256(packet.encode()).hexdigest(),'report_sha256':hashlib.sha256((p/'report.json').read_bytes()).hexdigest(),'free_bytes':os.statvfs(p).f_bavail*os.statvfs(p).f_frsize},indent=2))
