import datetime,json,pathlib,subprocess
p=pathlib.Path(__file__).parent
head="3c637b228449e9345d8bba31ea2d7d449f8a2b8f"
ids=[34449322494,34449322415,34449322222,34449322223]
receipt=p/"diff-service-recovery-receipt.json"
assert not receipt.exists(),"Recovery already attempted; inspect receipt, never replay blindly."
assert (p/"head3c-ci-incident-root-verification.json").exists()
record={"at":datetime.datetime.now(datetime.timezone.utc).isoformat(),"head":head,"reason":"Confirmed changed-file service failure; endpoint now healthy, rerun only completed failed jobs once.","steps":[]}
def save():receipt.write_text(json.dumps(record,indent=2)+"\n")
def api(path):
 z=subprocess.run(["/opt/homebrew/bin/gh","api",path],capture_output=True,text=True)
 if z.returncode:raise RuntimeError(z.stderr[:400])
 return json.loads(z.stdout)
b=api("rate_limit")["resources"];assert b["core"]["remaining"]>100
pr=api("repos/sparq-org/sparq/pulls/6478");assert pr["state"]=="open" and not pr["draft"] and pr["head"]["sha"]==head
files=api("repos/sparq-org/sparq/pulls/6478/files?per_page=100");assert len(files)==10 and pr["changed_files"]==10
record["budget"]=b;record["healthy_file_names"]=[x["filename"] for x in files]
for rid in ids:
 run=api("repos/sparq-org/sparq/actions/runs/"+str(rid))
 assert run["head_sha"]==head and run["event"]=="pull_request" and run["status"]=="completed" and run["conclusion"]=="failure" and run["run_attempt"]==1
 pr=api("repos/sparq-org/sparq/pulls/6478");assert pr["state"]=="open" and pr["head"]["sha"]==head
 step={"run":rid,"previous_attempt":run["run_attempt"],"name":run["name"],"request_started_at":datetime.datetime.now(datetime.timezone.utc).isoformat()};record["steps"].append(step);save()
 z=subprocess.run(["/opt/homebrew/bin/gh","api","repos/sparq-org/sparq/actions/runs/"+str(rid)+"/rerun-failed-jobs","--method","POST"],capture_output=True,text=True)
 step.update({"exit_code":z.returncode,"stdout":z.stdout,"stderr":z.stderr});save();assert z.returncode==0,"Recovery failed/uncertain; no automatic repeat."
record["completed_at"]=datetime.datetime.now(datetime.timezone.utc).isoformat();save();print(json.dumps({"head":head,"accepted_failed_job_reruns":ids,"live_runs_untouched":True}))
