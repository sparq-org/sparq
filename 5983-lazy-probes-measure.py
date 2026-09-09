"""[GPT-6 Astra] Allocation controls first, then one fixed fourteen-point matrix."""
from pathlib import Path
import hashlib
import json
import os
import subprocess
import time

out = Path(__file__).parent
(out/'raw').mkdir(exist_ok=True)
env = dict(os.environ,RAYON_NUM_THREADS='1')
metrics = ['allocs','reallocs','requested_bytes','peak_live_delta']

def invoke(variant,case,k,mode,phase):
    expected = 'indexed' if variant in ['new','dc','eager'] and case not in ['drained','drained-overlay','ties','ties-overlay'] else 'fallback'
    binary = out/'bins'/f'{variant}-{"count" if mode=="count" or variant=="eager" else "timing"}'
    command=[str(binary),'verify' if mode=='verify' else 'measure',case,'50000',str(k),'0' if mode=='verify' else '2','1' if mode=='verify' else ('3' if mode=='count' else '7'),expected]
    name=f'{phase}-{case}-k{k}-{variant}-{mode}'
    before=time.time()
    result=subprocess.run(command,capture_output=True,text=True,timeout=120,env=env)
    raw=out/'raw'/(name+'.txt');raw.write_text(result.stdout+result.stderr)
    rows=[]
    for line in result.stdout.splitlines():
        if not line.startswith('kind='):continue
        row={}
        for field in line.split('\t'):
            key,value=field.split('=',1)
            try:value=int(value)
            except ValueError:pass
            row[key]=value
        rows.append(row)
    record=dict(name=name,variant=variant,case=case,n=50000,k=k,mode=mode,command=command,exit_code=result.returncode,started_unix=before,wall_seconds=time.time()-before,binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),load_average=os.getloadavg(),rows=rows,raw=str(raw))
    assert result.returncode==0, f'{name} failed; stop without retry: {raw}'
    assert all(r.get('oracle','pass')=='pass' for r in rows)
    print(json.dumps({x:record[x] for x in ['name','exit_code','wall_seconds']}),flush=True)
    return record

def counts(record):
    rows=[r for r in record['rows'] if r['kind']=='sample'];assert len(rows)==3
    assert len({tuple(r[m] for m in metrics) for r in rows})==1
    return {m:rows[0][m] for m in metrics}

control=[];checks=[]
for case in ['base','drained','drained-overlay']:
    pair={}
    for variant in ['new','eager']:
        control.append(invoke(variant,case,1,'verify','control'))
        record=invoke(variant,case,1,'count','control');control.append(record);pair[variant]=counts(record)
        (out/'allocation-control.json').write_text(json.dumps(dict(records=control,checks=checks),indent=2)+'\n')
    if case=='base':
        assert pair['new']==pair['eager'], 'Positive no-miss control must have equal allocation work'
        checks.append(dict(case=case,positive_control='identical allocation metrics',metrics=pair['new']))
    else:
        seed=next(r['seed_rows'] for r in record['rows'] if r['kind']=='fixture')
        minimum=4*seed*4
        assert pair['eager']['requested_bytes']-pair['new']['requested_bytes']>=minimum, 'Avoid four later subject vectors'
        assert pair['new']['allocs']<pair['eager']['allocs']
        checks.append(dict(case=case,seed_rows=seed,minimum_avoided_bytes=minimum,new=pair['new'],eager=pair['eager'],avoided_requested_bytes=pair['eager']['requested_bytes']-pair['new']['requested_bytes'],avoided_allocations=pair['eager']['allocs']-pair['new']['allocs']))
(out/'allocation-control.json').write_text(json.dumps(dict(records=control,checks=checks,passed=True),indent=2)+'\n')
print('Allocation controls passed; beginning fixed matrix',flush=True)

cases=[('base',1),('base',512),('overlay',1),('overlay',512),('drained',1),('drained-overlay',1),('ties',1),('ties-overlay',1),('prefix128',1),('prefix128-overlay',1),('prefix800',1),('prefix800-overlay',1),('intermittent',1),('intermittent-overlay',1)]
variants=['new','dc','disabled','main'];records=[]
for index,(case,k) in enumerate(cases):
    for variant in variants[index%4:]+variants[:index%4]:
        for mode in ['verify','timing','count']:
            records.append(invoke(variant,case,k,mode,'matrix'))
            (out/'measurements.json').write_text(json.dumps(records,indent=2)+'\n')
print(json.dumps({'completed_processes':len(records),'matrix_query_samples':sum(sum(r['kind']=='sample' for r in x['rows']) for x in records),'control_processes':len(control)}),flush=True)
