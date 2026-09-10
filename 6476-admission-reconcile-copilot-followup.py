import datetime,json,pathlib,subprocess
p=pathlib.Path(__file__).parent
head="3c637b228449e9345d8bba31ea2d7d449f8a2b8f"
record=p/"copilot-followup-reconcile-receipt.json"
assert not record.exists(),"Already attempted; inspect receipts before resuming."
assert json.loads((p/"copilot-followup-push-receipt.json").read_text())["exit_code"]==0
assert json.loads((p/"opus-copilot-result.json").read_text())["verdict"]=="approve_delta_for_ci"
receipt={"started_at":datetime.datetime.now(datetime.timezone.utc).isoformat(),"head":head,"steps":[]}
def save():record.write_text(json.dumps(receipt,indent=2)+"\n")
def call(*args):
 z=subprocess.run(["/opt/homebrew/bin/gh",*args],capture_output=True,text=True)
 if z.returncode:raise RuntimeError(z.stderr[:500])
 return z.stdout
def api(*args):
 x=json.loads(call("api",*args));assert not (isinstance(x,dict) and x.get("errors")),x
 return x
query='query{rateLimit{remaining resetAt} repository(owner:"sparq-org",name:"sparq"){pullRequest(number:6478){id state isDraft baseRefName headRefOid mergeQueueEntry{state} labels(first:100){nodes{name}} reviewThreads(first:100){pageInfo{hasNextPage} nodes{id isResolved comments(last:10){pageInfo{hasPreviousPage} nodes{databaseId body author{login} commit{oid}}}}}}}}'
def fresh():
 x=api("graphql","-f","query="+query);assert x["data"]["rateLimit"]["remaining"]>50
 pr=x["data"]["repository"]["pullRequest"]
 assert pr["state"]=="OPEN" and not pr["isDraft"] and pr["headRefOid"]==head and pr["baseRefName"]=="main" and pr["mergeQueueEntry"] is None
 assert not any(l["name"].startswith(("review:needs","review:changes","review:blocked")) for l in pr["labels"]["nodes"])
 assert not pr["reviewThreads"]["pageInfo"]["hasNextPage"]
 return pr
b=api("rate_limit")["resources"];assert min(b[x]["remaining"] for x in ["core","graphql"])>50
pr=fresh();assert {x["id"] for x in pr["reviewThreads"]["nodes"]}=={"PRRT_kwDOSz3qKM6g9YwV","PRRT_kwDOSz3qKM6g9Ywu"}
receipt["initial"]=pr;save()
call("pr","edit","6478","--repo","sparq-org/sparq","--body-file",str(p/"pr-copilot-followup-body.md"));receipt["steps"].append({"body_edited":True});save()
replies=json.loads((p/"copilot-followup-replies.json").read_text())
for tid,body in replies.items():
 pr=fresh();t=next(x for x in pr["reviewThreads"]["nodes"] if x["id"]==tid);assert not t["isResolved"] and not t["comments"]["pageInfo"]["hasPreviousPage"]
 assert len(t["comments"]["nodes"])==1 and t["comments"]["nodes"][0]["author"]["login"]=="copilot-pull-request-reviewer"
 request={"query":"mutation($input:AddPullRequestReviewThreadReplyInput!){addPullRequestReviewThreadReply(input:$input){comment{id url}}}","variables":{"input":{"pullRequestReviewThreadId":tid,"body":body}}}
 q=p/("copilot-followup-reply-"+tid+".json");q.write_text(json.dumps(request)+"\n");out=api("graphql","--input",str(q));receipt["steps"].append({"thread":tid,"reply":out});save()
 pr=fresh();t=next(x for x in pr["reviewThreads"]["nodes"] if x["id"]==tid);assert len(t["comments"]["nodes"])==2 and t["comments"]["nodes"][-1]["body"]==body
 out=api("graphql","-f","query=mutation{resolveReviewThread(input:{threadId:\""+tid+"\"}){thread{id isResolved}}}");receipt["steps"].append({"thread":tid,"resolve":out});save()
pr=fresh();assert all(t["isResolved"] for t in pr["reviewThreads"]["nodes"])
requested=api("repos/sparq-org/sparq/pulls/6478/requested_reviewers")
if not any(x["login"]=="copilot-pull-request-reviewer" for x in requested["users"]):
 payload=p/"copilot-followup-rereview-input.json";payload.write_text(json.dumps({"reviewers":["copilot-pull-request-reviewer"]})+"\n")
 out=api("repos/sparq-org/sparq/pulls/6478/requested_reviewers","--method","POST","--input",str(payload));receipt["steps"].append({"copilot_requested":True,"requested_reviewers":out.get("requested_reviewers")});save()
receipt["final"]=fresh();receipt["completed_at"]=datetime.datetime.now(datetime.timezone.utc).isoformat();save();print(json.dumps({"head":head,"body_updated":True,"resolved_threads":2,"copilot_requested_or_pending":True}))
