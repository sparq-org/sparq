from pathlib import Path
import datetime,json,subprocess
A=Path(__file__).parent; R=A/'recover-paths-filter-receipt.json'; assert not R.exists(),'Reconcile prior recovery; never replay'
H='9ac4a59e96b1c69c8d617fbde9e98fe8c0436fed'; RUN=34511329019; JOB=102985915502
r={'started_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'recovery_cap':'One failed-job rerun; aggregate waits for relevant peers'}
def save(): R.write_text(json.dumps(r,indent=2)+'\n')
def api(*args):
 p=subprocess.run(['/opt/homebrew/bin/gh','api',*args],capture_output=True,text=True)
 if p.returncode: raise RuntimeError('Request failed; no retry: '+p.stderr[:800])
 return json.loads(p.stdout) if p.stdout.strip() else None
try:
 b=api('rate_limit')['resources']; assert min(b[k]['remaining'] for k in ['core','graphql'])>=100; r['budget']=b
 pr=api('repos/sparq-org/sparq/pulls/6486'); assert pr['state']=='open' and not pr['draft'] and pr['head']['sha']==H; r['pr']=pr
 files=api('repos/sparq-org/sparq/pulls/6486/files?per_page=100'); assert sorted(x['filename'] for x in files)==['bench/feature-off-declarations/6486.json','crates/sparq-core/src/lib.rs']; r['files_endpoint_recovered']=files
 run=api('repos/sparq-org/sparq/actions/runs/'+str(RUN)); assert run['head_sha']==H and run['status']=='completed' and run['conclusion']=='failure' and run['run_attempt']==1 and run['event']=='pull_request'; r['run']=run
 jobs=api('repos/sparq-org/sparq/actions/runs/'+str(RUN)+'/jobs?filter=latest&per_page=100'); assert jobs['total_count']==len(jobs['jobs']); failed=[j['id'] for j in jobs['jobs'] if j['conclusion']=='failure']; assert failed==[JOB] and all(j['status']=='completed' for j in jobs['jobs']); r['jobs']=jobs
 job=api('repos/sparq-org/sparq/actions/jobs/'+str(JOB)); assert job['run_id']==RUN and job['head_sha']==H and job['status']=='completed' and job['conclusion']=='failure'; r['immediate_job']=job
 current=api('repos/sparq-org/sparq/pulls/6486'); assert current['state']=='open' and not current['draft'] and current['head']['sha']==H; r['immediate_pr']=current; save()
 r['recovery_attempted']=True; save(); r['rerun_response']=api('repos/sparq-org/sparq/actions/jobs/'+str(JOB)+'/rerun','-X','POST'); r['accepted']=True; save()
 after=api('repos/sparq-org/sparq/actions/runs/'+str(RUN)); r['after_run']=after; r['completed_at']=datetime.datetime.now(datetime.timezone.utc).isoformat(); save()
 print(json.dumps({'run':RUN,'job':JOB,'failed_job_rerun_accepted':True,'after_status':after['status'],'after_attempt':after['run_attempt'],'aggregate_rerun':False}))
except Exception as e:
 r['error']=str(e); save(); raise
