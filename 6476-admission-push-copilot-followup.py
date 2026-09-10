import datetime,json,pathlib,subprocess
p=pathlib.Path(__file__).parent
w=p.parents[1]/"worktrees/issue6476"
head="3c637b228449e9345d8bba31ea2d7d449f8a2b8f"
parent="d07ca79f89e3a8945be46b516c3cd2f770ccd618"
receipt=p/"copilot-followup-push-receipt.json"
assert not receipt.exists(),"Push already attempted; inspect receipt and resume missing steps only."
review=json.loads((p/"opus-copilot-result.json").read_text())
assert review["reviewed_head"]==head and review["verdict"]=="approve_delta_for_ci" and not review["blocking_findings"]
def run(args):
 z=subprocess.run(args,capture_output=True,text=True)
 if z.returncode:raise RuntimeError(z.stderr[:500])
 return z.stdout
def api(*args):
 v=json.loads(run(["/opt/homebrew/bin/gh","api",*args]))
 assert not (isinstance(v,dict) and v.get("errors")),v
 return v
b=api("rate_limit")["resources"];assert min(b[x]["remaining"] for x in ["core","graphql"])>50
assert run(["git","-C",str(w),"rev-parse","HEAD"]).strip()==head
assert not run(["git","-C",str(w),"status","--porcelain"]).strip()
query='query{repository(owner:"sparq-org",name:"sparq"){defaultBranchRef{target{oid}} pullRequest(number:6478){id state isDraft baseRefName headRefOid mergeQueueEntry{state} labels(first:100){nodes{name}} reviewThreads(first:100){pageInfo{hasNextPage} nodes{id comments(last:1){nodes{databaseId}}}}}}}'
f=api("graphql","-f","query="+query);pr=f["data"]["repository"]["pullRequest"]
assert pr["state"]=="OPEN" and not pr["isDraft"] and pr["baseRefName"]=="main" and pr["headRefOid"]==parent and pr["mergeQueueEntry"] is None
assert {x["name"] for x in pr["labels"]["nodes"]}<={"area:bench","area:sparq-engine","review:unreviewed"}
ts=pr["reviewThreads"];assert not ts["pageInfo"]["hasNextPage"]
assert {x["id"]:x["comments"]["nodes"][-1]["databaseId"] for x in ts["nodes"]}=={"PRRT_kwDOSz3qKM6g9YwV":3976252036,"PRRT_kwDOSz3qKM6g9Ywu":3976252068}
assert api("repos/sparq-org/sparq/git/ref/heads/codex/nested-query-budget")["object"]["sha"]==parent
(p/"copilot-followup-push-precheck.json").write_text(json.dumps(f,indent=2)+"\n")
z=subprocess.run(["git","-C",str(w),"push","--porcelain","origin",head+":refs/heads/codex/nested-query-budget"],capture_output=True,text=True)
receipt.write_text(json.dumps({"at":datetime.datetime.now(datetime.timezone.utc).isoformat(),"head":head,"parent":parent,"exit_code":z.returncode,"stdout":z.stdout,"stderr":z.stderr},indent=2)+"\n")
assert z.returncode==0,"Push failed/uncertain; do not retry without inspecting receipt."
print(json.dumps({"pushed":head,"exit":z.returncode}))
