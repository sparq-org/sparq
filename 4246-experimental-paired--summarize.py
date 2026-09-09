from pathlib import Path
import json,statistics as s
p=Path(__file__).parent
names=['main-time','candidate-time','main-count','candidate-count']
data={name:[json.loads(l) for l in (p/(name+'.jsonl')).read_text().splitlines()] for name in names}
keys=sorted({(r['case'],r['warm_perms'],r['generation']) for r in data['main-time'] if r['kind']=='lifecycle'})
def stat(values):
 return dict(n=len(values),min=min(values),median=s.median(values),max=max(values),mean=s.mean(values),sample_stdev=s.stdev(values) if len(values)>1 else 0)
result=[]
for case,perms,generation in keys:
 row=dict(case=case,warm_perms=perms,generation=generation)
 for name in names:
  group=[r for r in data[name] if r.get('case')==case and r['warm_perms']==perms and r['generation']==generation]
  assert len(group)==(3 if name.endswith('count') else 7)
  stats={k:stat([r[k] for r in group]) for k in ['elapsed_ns','allocs','reallocs','requested_bytes','peak_live_delta_bytes','live_before','live_after','retained_overlay_reported_heap','initial_overlay_reported_heap','rss_before','rss_after']}
  stats['phase_ns']=[stat([r['phase_ns'][i] for r in group]) for i in range(3)]
  stats['live_growth']=stat([r['live_after']-r['live_before'] for r in group])
  row[name]=stats
 row['candidate_main_median_ratio']=row['candidate-time']['elapsed_ns']['median']/row['main-time']['elapsed_ns']['median']
 row['timing_ranges_disjoint']=row['candidate-time']['elapsed_ns']['min']>row['main-time']['elapsed_ns']['max'] or row['candidate-time']['elapsed_ns']['max']<row['main-time']['elapsed_ns']['min']
 result.append(row)
report=dict(recorded_windows=sum(len(v)-1 for v in data.values()),fixtures={k:v[0] for k,v in data.items()},points=result)
(p/'summary.json').write_text(json.dumps(report,indent=2)+'\n')
lines=['| Case | Warm perms | Generation | Main / candidate median ns | Ratio | Main / candidate requested bytes | Candidate retained overlay heap |','|---|---:|---:|---:|---:|---:|---:|']
for r in result:
 def val(name,k):return r[name][k]['median']
 lines.append(f"| {r['case']} | {r['warm_perms']} | {r['generation']} | {val('main-time','elapsed_ns'):g} / {val('candidate-time','elapsed_ns'):g} | {r['candidate_main_median_ratio']:.3f} | {val('main-count','requested_bytes'):g} / {val('candidate-count','requested_bytes'):g} | {val('candidate-count','retained_overlay_reported_heap'):g} |")
(p/'summary.md').write_text('\n'.join(lines)+'\n')
print('windows',report['recorded_windows'],'points',len(result))
print('\n'.join(lines))
