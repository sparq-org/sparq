# [GPT-6 Astra] Freeze a review packet; no model invocation or remote action.
from pathlib import Path
import ast, hashlib, json, re, subprocess
E=Path(__file__).resolve().parent; W=Path('/private/tmp/sparq-pr6049/.throughput-monitor/worktrees/pr6095')
sha=lambda b:hashlib.sha256(b).hexdigest()
metadata=json.loads((E/'commit-metadata.json').read_text())
controls=json.loads((E/'controls.json').read_text())
commands=json.loads((E/'commands.json').read_text())
assert all(r['calibrated_kill'] for r in controls)
assert subprocess.check_output(['git','status','--porcelain'],cwd=W)==b''
assert subprocess.check_output(['git','diff',metadata['parents'][1]],cwd=W)==(E/'full.diff').read_bytes()
report={
 'head':metadata['head'],'parents':metadata['parents'],'base':metadata['parents'][1],
 'branch':metadata['branch'],'worktree':str(W),'clean':True,
 'scope':'Automatic, conflict-free merge of current main into the original PR6095 ancestry. No authored source changes; only the inherited two-file +68/-3 delta remains against main.',
 'provenance':'Original source commit 0883 retains sparq-orchestrator[bot] authorship and Claude Opus 5 coauthor. Actual OpenAI GPT-6 Astra xhigh performed recovery composition, validation, controls and this evidence; merge commit carries honest Astra trailer.',
 'contract':'Reject a declaration key that is the tail of a word/underscore/hyphen token; preserve genuine crate/surface declarations and nonempty complete scope-property coverage. This is lexical boundary repair, not a new requirement for declarations to begin a line. Standalone fields following whitespace still work.',
 'preserved':'Production top-level AST except _DECL is identical to main: full rule table, classifier tier order, candidate census, plan/apply semantics, unknown-label global zero-write guard, pacing/budgets. PR6473 diagnostic tests and ANCHORED_ONLY AST also identical. All other files equal main.',
 'tests':{'classifier':47,'classifier_selftest_checks':sum(bool(re.match(r'^ok   ',x)) for x in (E/'classifier-selftest.log').read_text().splitlines()),'migration_selftest_checks':sum(bool(re.match(r'^  ok   ',x)) for x in (E/'migration-selftest.log').read_text().splitlines()),'controls':len(controls),'control_tests_each':47,'controls_killed':sum(r['calibrated_kill'] for r in controls),'survivors':0,'control_errors':0},
 'coverage':'Stdlib trace with called/uncalled calibration before controls. All new test/helper statement starts and classify statement starts executed. Unchanged declared_areas path-fragment hit line439 is not executed by this focused suite; failure-report append lines969/986 naturally unexecuted in green baseline. Not full branch/line coverage claim.',
 'preflight':'FAIL, only privacy-claims: installed Bash3 has no mapfile at check-privacy-claims.sh:92. G1/G2/G6 and guard-untested pass; no-perf-numbers/readme-template skip because no matching paths. No gate edits or bypass. LinuxCI must execute this check.',
 'review_state':'Remote PR6095 remains OPEN DRAFT at0883, review:needs/area:ci/trust-surface; no remote calls or mutations in this recovery. Old cancelled auto-arm source check did not prove a source defect. Gate-green bot hold remains until normal exact-head CI and root reconciliation. No rearm/approval/hold clearance.',
 'trust':'Issue authors control title/body. T0 is a routing declaration tier, not authentication or review/admission evidence. This patch narrows accidental declaration matching; it does not grant label/review/queue authority or sanitize arbitrary prose generally.',
 'limits':['Only scoped local Python checks; no full workspace/CI claim.', 'Local Python3.14.5 versus docs-quality Python3.12; authoritative Linux gate remains.', 'Scope properties catch empty/truncated helpers, but do not claim arbitrary duplicate/overlapping helper corruption proof.', 'Real crate inventory comes from this checkout; no live board/classifier replay.', 'No fresh independent review by this worker; root will obtain actual Opus.'],
 'recommended_next_action':'Review frozen exact head with actual Opus; then root updates the existing PR through normal publication/CI/hold reconciliation. No new PR or issue required.'
}
(E/'report.json').write_text(json.dumps(report,indent=2)+'\n')
(E/'commands-and-controls.txt').write_text('Validation runner: /opt/homebrew/bin/python3 '+str(E/'run-validation.py')+'\nControls runner: PYTHONDONTWRITEBYTECODE=1 /opt/homebrew/bin/python3 '+str(E/'run-controls.py')+'\nComposition: /opt/homebrew/bin/python3 '+str(E/'verify-composition.py')+'\nAll shell invocations /bin/zsh login:false, cwd '+str(W)+'\nNo Cargo, installs, live API, dispatcher, model or benchmark calls.\n')
parts=['# PR6095 / issue4567 recovery — exact-head review packet\n\n[GPT-6 Astra] Recovery and validation; inherited source remains Claude Opus 5. Please independently assess only the actual inherited repair and its composition with main.\n', '## Scope, provenance, results and limitations\n\n```json\n'+json.dumps(report,indent=2)+'\n```\n']
def add(title,text,kind='text'):
 parts.append('\n## '+title+'\n\n```'+kind+'\n'+text.rstrip()+'\n```\n')
add('Exact final delta against main', (E/'full.diff').read_text(),'diff')
add('Original author and recovery commit', (E/'original-commit.txt').read_text()+(E/'commit.txt').read_text())
add('Mechanical composition proof', (E/'composition.json').read_text(),'json')
def excerpt(file,ranges):
 lines=(W/file).read_text().splitlines()
 return '\n'.join('\n'.join(f'{i}: {lines[i-1]}' for i in range(a,b+1)) for a,b in ranges)
add('Production boundary, declaration parser, full tier dispatch and fallback',excerpt('scripts/triage-area.py',[(393,484)]),'python')
add('Production read/write callers, global guard and CLI budget boundary',excerpt('scripts/triage-area.py',[(571,620),(730,824)]),'python')
add('Test loader, actual rule witness generator and partition helpers',excerpt('scripts/tests/test_triage_area.py',[(42,179)]),'python')
add('Complete scope-property class (new hygiene tests are complete in raw diff)',excerpt('scripts/tests/test_triage_area.py',[(894,1011)]),'python')
# Preserved PR6473 test class remains in frozen source; AST equality and executed suite recorded.
log=(E/'classifier-suite.log').read_text()
add('Actual classifier output excerpts (complete log in bundle)', '\n'.join(line for line in log.splitlines() if any(name in line for name in ('test_a_declaration_', 'test_a_real_declaration_', 'test_a_title_scoped_rule_', 'test_a_text_scoped_rule_'))) + '\n' + '\n'.join(log.splitlines()[-5:]))
add('Embedded self-tests (actual output excerpts; complete logs in bundle)', (E/'classifier-selftest.log').read_text() + (E/'migration-selftest.log').read_text().splitlines()[-1] + '\nMigration: ' + str(report['tests']['migration_selftest_checks']) + ' actual passing assertion lines counted.')
add('Author preflight actual output', (E/'author-preflight.log').read_text())
add('Calibrated control results', (E/'controls.json').read_text(),'json')
for row in controls:
 name=row['name']; log=(E/(name+'.log')).read_text()
 # Exact failure section, omitting repeated successful test lines (full logs hashed).
 failure=log[log.index('='*70):] if '='*70 in log else log
 if name in ('disable-declaration','empty-title-partition','empty-text-partition','partial-title-partition'):
  failure=failure.split('='*70)[1] + '\n' + '\n'.join(log.splitlines()[-4:])
 # Other failure bodies retained verbatim.
 add('Control '+name+' — exact mutation and failure output',(E/(name+'.diff')).read_text()+failure)
# Full actual executable harness is separately frozen and manifested.
# Complete line-event counts and statement-start summary separately frozen.
add('Workflow execution surface (unchanged main)',excerpt('.github/workflows/docs-quality.yml',[(144,155),(175,188),(625,628)])+'\n'+excerpt('.github/workflows/triage-area.yml',[(110,123)]),'yaml')
add('Commands and runtime',(E/'commands-and-controls.txt').read_text()+(E/'python-version.log').read_text()+(E/'bash-version.log').read_text())
parts.append('\n## Omitted context\n\nThe full 45KB production script, complete test file, migration self-test source, preflight source and unchanged workflow files are frozen under sources/ and hashed in manifest.json. Unchanged bulk RULES and unrelated test classes are omitted from this packet; the complete table was executed by the scope witness properties. All six control logs are complete in the bundle; repeated passing lines are omitted here, and only the first failure body from each multi-failure control is included (complete failure IDs/counts above). Classifier and migration logs are excerpted explicitly. The exact executed run-controls.py and detailed coverage.json/coverage-summary.json are separately frozen. The unchanged PR6473 diagnostic test class is not duplicated; its AST equality and all 47 passing tests are recorded. New hygiene method bodies appear in full in the raw diff. No transcripts or external model output are included. Historic review/hold state is root-provided assessment evidence, not a fresh gate or review.\n')
packet=''.join(parts)
(E/'review-packet.md').write_text(packet)
print('packet_bytes',len(packet.encode()))
