from pathlib import Path
import datetime,json,subprocess
R=Path('/private/tmp/sparq-pr6049/.throughput-monitor'); A=R/'direct-6480/admission'; W=R/'worktrees/issue6480'
HEAD='0c6a780593a9b3381fb158e426519a2a6d8d17f9'; OLD='26d139520f07f1ceacafbacbeb9991de371e2b53'; BASE='781f667c19a8ebb779cfccb24b05ea432360b025'; BRANCH='codex/next-security-patch'
receipt={'started_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
def save(): (A/'followup-publication-receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
def command(args):
 p=subprocess.run(args,cwd=W,text=True,capture_output=True)
 if p.returncode: raise RuntimeError(f'Command failed ({p.returncode}): {p.stderr}')
 return p.stdout
def api(*args):
 s=command(['/opt/homebrew/bin/gh','api',*args]); return json.loads(s) if s.strip() else None
def target(expected):
 p=api('repos/sparq-org/sparq/pulls/6481')
 assert p['state']=='open' and not p['draft'] and p['head']['sha']==expected and p['head']['ref']==BRANCH and p['base']['ref']=='main'
 assert p['auto_merge'] is None
 assert not {'review:pass','review:needs','review:needs-owner'} & {x['name'] for x in p['labels']}, 'Reconcile protected review evidence or hold before mutation'
 return p
try:
 assert not (A/'followup-publication-receipt.json').exists(), 'Inspect existing mutation receipt; never blindly replay'
 budget=api('rate_limit'); assert budget['resources']['core']['remaining']>=40 and budget['resources']['graphql']['remaining']>=10, 'Fail closed: insufficient request reserve'
 receipt['budget']=budget['resources']
 rev=json.loads((A/'opus-followup-result.json').read_text()); assert rev['reviewed_head']==HEAD and rev['verdict']=='approve_for_ci' and not rev['blocking_findings']
 disposition=json.loads((A/'opus-followup-root-disposition.json').read_text()); assert disposition['approved_for_publication'] is True and disposition['head']==HEAD
 assert command(['git','rev-parse','HEAD']).strip()==HEAD and command(['git','branch','--show-current']).strip()==BRANCH
 assert command(['git','rev-parse','HEAD^']).strip()==OLD and not command(['git','status','--porcelain'])
 assert command(['git','diff','--name-only',BASE,HEAD]).splitlines()==['.github/workflows/js.yml','gui/app/package.json','package-lock.json','scripts/tests/test_js_wasm_pack_install.py','site/package.json','supply-chain/npm-advisories.md']
 registry=api('repos/jeswr/agent-account-registry/actions/permissions');assert registry['enabled'] is False;receipt['registry']=registry
 main=api('repos/sparq-org/sparq/git/ref/heads/main');assert main['object']['sha']==BASE;receipt['main']=main
 graph=api('graphql','--input',str(A/'monitor-followup-query.json'));assert not graph.get('errors'); gp=graph['data']['repository']['pullRequest'];assert gp['headRefOid']==OLD and gp['mergeQueueEntry'] is None;receipt['prepush_graph']=graph
 runs=api(f'repos/sparq-org/sparq/actions/runs?head_sha={OLD}&per_page=100');assert runs['total_count']<=100 and all(x['status']=='completed' for x in runs['workflow_runs']), 'Wait for all old-head evidence to finish before push'
 receipt['old_runs']=runs
 remote=command(['git','ls-remote','--heads','origin','refs/heads/'+BRANCH]);assert remote.split()[0]==OLD
 receipt['prepush']=target(OLD);save()
 p=subprocess.run(['git','push','origin',HEAD+':refs/heads/'+BRANCH],cwd=W,text=True,capture_output=True);receipt['push']={'exit_code':p.returncode,'stdout':p.stdout,'stderr':p.stderr};save();assert p.returncode==0, 'Push failed; inspect receipt, no repeated push'
 ref=api('repos/sparq-org/sparq/git/ref/heads/'+BRANCH);receipt['ref']=ref;save();assert ref['object']['sha']==HEAD, 'Inspect GitHub propagation without repeating push'
 receipt['pre_metadata']=target(HEAD);save()
 updated=api('repos/sparq-org/sparq/pulls/6481','-X','PATCH','--input',str(A/'pr-followup-metadata-input.json'));receipt['metadata']=updated;save();assert updated['head']['sha']==HEAD
 issue=api('repos/sparq-org/sparq/issues/6480');assert issue['state']=='open'
 comment=api('repos/sparq-org/sparq/issues/comments/5616304836');assert comment['user']['login']=='jeswr' and comment['issue_url'].endswith('/issues/6480') and 'SPARQ agent' in comment['body']
 receipt['pre_comment']={'issue':issue,'comment':comment};save()
 result=api('repos/sparq-org/sparq/issues/comments/5616304836','-X','PATCH','--input',str(A/'issue6480-followup-comment-input.json'));receipt['issue_comment']=result;receipt['completed_at']=datetime.datetime.now(datetime.timezone.utc).isoformat();save()
 print(json.dumps({'head':HEAD,'pr':updated['html_url'],'title':updated['title'],'comment':result['html_url'],'completed_at':receipt['completed_at']}))
except Exception as e:
 receipt['error']=str(e);save();raise
