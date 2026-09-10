from pathlib import Path
import subprocess,json,datetime,hashlib
A=Path(__file__).parent;H='fe3284199db0831f353d2ae401b8c69472764904';C=5618039976;P=A/'remaining-original-comment-receipt.json';assert not P.exists(),'Reconcile existing edit before repeat'
receipt={'started_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
def save():P.write_text(json.dumps(receipt,indent=2)+'\n')
def api(path,name,method='GET',inp=None):
 args=['/opt/homebrew/bin/gh','api',path,'-X',method]
 if inp:args+=['--input',str(inp)]
 p=subprocess.run(args,capture_output=True,text=True)
 if p.returncode:raise RuntimeError('Request failed; no retry: '+p.stderr[:500])
 j=json.loads(p.stdout);(A/('remaining-original-comment-'+name+'.json')).write_text(json.dumps(j,indent=2)+'\n');return j
try:
 b=api('rate_limit','budget');assert b['resources']['core']['remaining']>=20
 i=api('repos/sparq-org/sparq/issues/5183','issue');assert i['state']=='open'
 pr=api('repos/sparq-org/sparq/pulls/6482','pr');assert pr['head']['sha']==H
 c=api(f'repos/sparq-org/sparq/issues/comments/{C}','before');assert c['user']['login']=='jeswr' and c['issue_url'].endswith('/issues/5183')
 expected=json.loads((A/'original-seed-comment-result.json').read_text());assert c['body']==expected['body'],'Existing comment changed; reconcile instead of overwriting'
 marker='<!-- sparq-direct-5183-remaining-original-v1 -->';assert marker not in c['body']
 extra='''\n\n### Remaining original cases and PR #6482\n\nAll seven remaining seed markers were reproduced once each on both `d41ec9f` and `781f667`, using the unchanged original generator and pinned dependency artifacts. Failure bodies and reconstructed sequences match across revisions. These are deterministic reconstructions: CI retained only the seven seed markers, not their full request text.\n\nThe failing zero-based indices are `4141222535:7`, `4141222571:9`, `4141222576:8`, `4141222599:5`, `4141222612:6`, `4141222617:5`, and `4141222622:4`. Every sequence inserts both `8` and `008` at `s2/p1` before the fresh-blank-node INSERT. Each observed delta is consistent with the same reference lexical-collapse/cardinality mechanism as the first seed. Actual independent Claude Opus5 xhigh review supports that grouping with qualifications; these seven did not add direct per-step WHERE-count instrumentation.\n\nThe original comparator explicitly adjudicates earlier lexical-only differences by normalization; the later extra blank-node structure remains a mismatch. Thus earlier reference comparisons passing does not mean their raw terms were identical. Strict Sparq-versus-Sparq comparison runs first and remains separate.\n\nThe repaired generator and stricter lexical adjudicator are now in [PR #6482](https://github.com/sparq-org/sparq/pull/6482), with the exact first historical sequence retained as a Sparq-only regression. Local Rust 2021 validation and independent source review passed; normal protected CI and merge remain required. The identical-lexical strict-comparison coverage gap raised during review is tracked separately in #6483.\n\nThis issue stays open pending the landed repair. The trailing ADD requests in `4141222612` and `4141222617` were not executed after their first mismatch. No claim is made that the original lexical-alias inputs now pass against the lossy reference, that engine semantics changed, or that passing the same numeric seeds under the new generator replays the originals.\n'''
 body=c['body']+extra+'\n'+marker+'\n';inp=A/'remaining-original-comment-input.json';inp.write_text(json.dumps({'body':body},indent=2)+'\n');receipt['expected_before_sha256']=hashlib.sha256(c['body'].encode()).hexdigest();save()
 out=api(f'repos/sparq-org/sparq/issues/comments/{C}','result','PATCH',inp);assert out['body']==body;receipt.update({'completed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'url':out['html_url'],'body_sha256':hashlib.sha256(body.encode()).hexdigest()});save();print(json.dumps({'url':receipt['url'],'body_sha256':receipt['body_sha256']}))
except Exception as e:receipt['error']=str(e);save();raise
