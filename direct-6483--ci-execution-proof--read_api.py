import json,sys,subprocess,time,pathlib
p=pathlib.Path(__file__).parent
name,endpoint=sys.argv[1:]
receipts=p/"requests.jsonl"
n=len(receipts.read_text().splitlines()) if receipts.exists() else 0
assert n<20, "request cap"
if name!="budget":
 b=json.loads((p/"budget.json").read_text())["resources"]["core"];assert b["remaining"]>0
 assert not (p/"STOP").exists(), "prior request failed"
a=["/opt/homebrew/bin/gh","api",endpoint];t=time.time()
with (p/(name+".json" if not name.endswith("log") else name+".txt")).open("wb") as o,(p/(name+".stderr")).open("wb") as e:
 c=subprocess.run(a,stdout=o,stderr=e,timeout=60)
r={"argv":a,"start_unix":t,"elapsed_seconds":time.time()-t,"exit_code":c.returncode,"name":name}
with receipts.open("a") as f:f.write(json.dumps(r)+"\n")
if c.returncode:(p/"STOP").write_text(json.dumps(r))
print(json.dumps(r));sys.exit(c.returncode)
