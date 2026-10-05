// Fable soundness-verdict lane (Fable collaboration tier — umbrella #1111 "revisit-with-fable"). [OPUS-4.8]
//
// PURPOSE: route the OPEN honesty/soundness-sensitive PRs — the ones the cheap fleet and the
// autonomous scheduler deliberately LEAVE OPEN for a human/stronger judgment — through the
// expensive model, cheaply. A HAIKU recon assembles a compact evidence pack per PR so FABLE
// pays for JUDGMENT, not for scrolling diffs; FABLE returns one verdict per PR; the
// orchestrator arms strictly on the verdict fields.
//
//   Stage1  Enumerate  HAIKU  list open honesty/soundness PRs + assemble each evidence pack
//                             (diff summary + tests + audit-doc refs + sq-qhy4/ZK-MPC flags)
//   Stage2  Review     FABLE  adversarial soundness review, ONE call per PR (fan-out)
//   Stage3  Arm        glue   iterate the VERDICT objects: arm honest+recommend_arm+soundness_ok;
//                             HOLD honest=false and anything touching sq-qhy4 (external ZK audit)
//
// MODEL TIERS: opts.model drives the tier TODAY — the top-tier Review fan-out dispatches the
// FULL primary id via TOP_TIER_MODEL below (canonical TIER table: fable-architect-drain.js; the
// bare 'fable' alias still serves the claude-fable-5 downgrade tier — alias lag probe-proven,
// PR #3763), while the cheap stages keep the 'haiku' alias. opts.agentType names the
// role agent ('sparq-reviewer'), now APPLIED under .claude/agents/ (maintainer-authorized,
// commit c28d90e4); the agentType ref resolves to that role config and opts.model overrides
// the tier on top of it. (If the agent is ever removed, an unknown agentType falls back to the
// default toolset and the model override alone still drives the tier.)
//
// DURABLE: committed under .claude/workflows/. Re-run with:
//   Workflow({ name: "fable-soundness-verdict" })                                 // by label
//   Workflow({ name: "fable-soundness-verdict", args: { prs: ["<url>", ...] } })  // explicit
//
// SAFETY: every stage is READ-ONLY except the final `gh pr merge --auto` arm. Arming iterates
// the VERDICT OBJECTS (never a blind PR-number / CI-mergeStateStatus loop — arm-gate-on-verdict
// rule); an honest=false verdict or a PR touching the pending EXTERNAL ZK audit (sq-qhy4) is
// HELD OPEN for the maintainer regardless of the other fields.

export const meta = {
  name: 'fable-soundness-verdict',
  description: 'Fable-tier review lane for open honesty/soundness PRs: HAIKU assembles a per-PR evidence pack, FABLE returns one soundness verdict per PR, and the orchestrator arms strictly on the verdict fields (holding sq-qhy4 / honest=false).',
  whenToUse: 'Run to clear the queue of open honesty/soundness-sensitive PRs the cheap fleet left for stronger judgment, paying the expensive model only for the verdict.',
  phases: [
    { title: 'Enumerate', detail: 'HAIKU: list open honesty/soundness PRs + assemble each evidence pack' },
    { title: 'Review', detail: 'FABLE adversarial soundness review, one call per PR (fan-out)' },
    { title: 'Arm', detail: 'iterate verdict objects: arm clean, hold sq-qhy4 / honest=false for the maintainer' },
  ],
}

const ENUM_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    prs: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: {
          number: { type: 'number' },
          url: { type: 'string' },
          surface: { type: 'string' },
          title: { type: 'string' },
          // Compact evidence pack so Fable pays for judgment, not diff-scrolling.
          evidence: {
            type: 'object',
            additionalProperties: false,
            properties: {
              diff_summary: { type: 'string' },
              tests: { type: 'array', items: { type: 'string' } },
              audit_docs: { type: 'array', items: { type: 'string' } },
              zk_mpc_surface: { type: 'boolean' },
              touches_sq_qhy4: { type: 'boolean' },
            },
            required: [],
          },
        },
        required: ['url'],
      },
    },
  },
  required: ['prs'],
}

// Fable soundness verdict (READ-ONLY): the orchestrator arms from these fields.
const SOUND_VERDICT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    honest: { type: 'boolean' },
    soundness_ok: { type: 'boolean' },
    recommend_arm: { type: 'boolean' },
    concerns: { type: 'array', items: { type: 'string' } },
    hold_reason: { type: 'string' },
  },
  required: ['honest', 'recommend_arm'],
}

const ARM_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    armed: { type: 'boolean' },
    pr: { type: 'string' },
    error: { type: 'string' },
  },
  required: ['armed'],
}

const LABELS = (args && Array.isArray(args.labels)) ? args.labels : ['soundness', 'honesty']
const EXPLICIT_PRS = (args && Array.isArray(args.prs)) ? args.prs : null

// Top-tier DISPATCH id — the Opus 5 primary BY FULL ID (single-sourced canonical TIER
// table: .claude/workflows/fable-architect-drain.js). Full-ID dispatch serves exactly the
// requested model (probe-proven, PR #3763); the bare 'fable' alias would silently serve
// the claude-fable-5 downgrade tier, so it is never passed as opts.model here.
const TOP_TIER_MODEL = 'claude-opus-5'

function enumeratePrompt() {
  if (EXPLICIT_PRS) {
    return 'For EACH of these PRs, assemble a compact evidence pack: ' + JSON.stringify(EXPLICIT_PRS) + '. ' +
      'READ-ONLY (findings only; do NOT git-checkout, open no PR): for each, `gh pr view <url> --json number,url,title,labels,headRefName` and `gh pr diff <url> --name-only`. ' +
      'Summarise the diff in one or two sentences, list the tests it adds/changes, list any audit/threat-model/research docs it touches (e.g. research/threat-model.md, an audit record), and set zk_mpc_surface / touches_sq_qhy4 (true if it touches the ZK/MPC estate or the PENDING external-audit surface sq-qhy4). ' +
      'Return {prs:[{number,url,surface,title,evidence:{diff_summary,tests,audit_docs,zk_mpc_surface,touches_sq_qhy4}}]}.'
  }
  return 'Enumerate the OPEN honesty/soundness-sensitive PRs on `sparq-org/sparq` and assemble a compact evidence pack for each. ' +
    'READ-ONLY (findings only; do NOT git-checkout, open no PR). List candidates with `gh pr list --state open --json number,url,title,labels,headRefName` and select the ones whose labels or titles indicate honesty/soundness/ZK/MPC/security review is needed (labels: ' + JSON.stringify(LABELS) + '; or a title/diff making a soundness or privacy claim). ' +
    'For each selected PR: `gh pr diff <url> --name-only` (+ read the load-bearing hunks), summarise the diff in one or two sentences, list the tests it adds/changes, list any audit/threat-model/research docs it touches, and set zk_mpc_surface / touches_sq_qhy4 (true if it touches the ZK/MPC estate or the PENDING external-audit surface sq-qhy4). ' +
    'Keep the pack SMALL — Fable will read the pack, not the whole diff. If none qualify, return {prs:[]}. ' +
    'Return {prs:[{number,url,surface,title,evidence:{diff_summary,tests,audit_docs,zk_mpc_surface,touches_sq_qhy4}}]}.'
}

function reviewPrompt(p) {
  const ev = JSON.stringify(p.evidence || {})
  return 'You are the FABLE SOUNDNESS REVIEWER. Adversarially review OPEN PR ' + p.url + ' ("' + (p.title || '') + '", surface ' + (p.surface || '') + '). ' +
    'Evidence pack from recon (start here; pull the full diff only where the pack is insufficient): ' + ev + '. ' +
    'READ-ONLY: `gh pr diff ' + p.url + '` + `gh pr view ' + p.url + '` + the changed files (do NOT git-checkout, do NOT push, do NOT merge — you return a VERDICT the orchestrator acts on). ' +
    'Judge, non-sycophantically: (1) HONEST — no overclaim, every number traces to real evidence, work-box timings not presented as canonical, no hard-coded perf; (2) SOUND — the proof/argument/test actually establishes what it claims; (3) any ZK/MPC claim stays "research-grade / not externally audited" — the v1 verifier is internally re-audited but EXTERNAL accredited-cryptographer sign-off is PENDING (sq-qhy4) and MPC is semi-honest-only; an unqualified ZK/MPC soundness or privacy claim is a HARD not-honest. ' +
    'recommend_arm=true ONLY if honest AND soundness_ok AND low-risk. If it touches sq-qhy4 / the external-ZK-audit surface, set soundness_ok honestly but expect the orchestrator to HOLD it for the maintainer regardless. ' +
    'Return {honest, soundness_ok, recommend_arm, concerns:[...], hold_reason}.'
}

function armPrompt(v) {
  return 'Arm PR ' + v.pr + ' for the merge train: the Fable soundness reviewer returned honest=true, soundness_ok=true, recommend_arm=true, and it does NOT touch the pending external ZK audit (sq-qhy4). ' +
    'Run `gh pr merge ' + v.pr + ' --squash --auto` and confirm auto-merge latched. Do NOT force-merge; if the arm command errors, report it. ' +
    'Return {armed, pr:"' + v.pr + '", error}.'
}

// Stage1 — HAIKU enumerate + evidence packs (one read-only call).
phase('Enumerate')
const enumd = await agent(enumeratePrompt(), { label: 'enumerate', phase: 'Enumerate', schema: ENUM_SCHEMA, agentType: 'general-purpose', model: 'haiku' })
const prs = ((enumd && enumd.prs) || []).filter(p => p && p.url)
if (!prs.length) {
  log('no open honesty/soundness PRs to review')
  return { reviewed: 0, armed: [], held: [], skipped: [] }
}
log('enumerated ' + prs.length + ' PR(s) for Fable soundness review: ' + prs.map(p => p.url).join(', '))

// Stage2 — FABLE reviewer, ONE call per PR (fan-out). Read-only verdicts.
phase('Review')
const verdicts = await parallel(prs, (p) =>
  agent(reviewPrompt(p), { label: 'fable:' + (p.number || p.url), phase: 'Review', schema: SOUND_VERDICT_SCHEMA, agentType: 'sparq-reviewer', model: TOP_TIER_MODEL })
    .then(v => ({
      pr: p.url,
      surface: p.surface,
      honest: (v && v.honest) || false,
      soundness_ok: (v && v.soundness_ok) || false,
      recommend_arm: (v && v.recommend_arm) || false,
      concerns: (v && v.concerns) || [],
      hold_reason: (v && v.hold_reason) || '',
      touches_sq_qhy4: !!(p.evidence && p.evidence.touches_sq_qhy4),
    }))
)

// Stage3 — orchestrator glue: iterate the VERDICT OBJECTS (never a blind PR-number loop).
// Arm honest+soundness_ok+recommend_arm; HOLD honest=false and anything touching sq-qhy4.
phase('Arm')
const armed = []
const held = []
for (const v of verdicts) {
  const holdForAudit = v.touches_sq_qhy4
  const cleared = v.honest && v.soundness_ok && v.recommend_arm
  if (cleared && !holdForAudit) {
    const a = await agent(armPrompt(v), { label: 'arm:' + v.pr, phase: 'Arm', schema: ARM_SCHEMA, agentType: 'general-purpose', model: 'haiku' })
    if (a && a.armed) { armed.push(v.pr); continue }
    held.push({ pr: v.pr, reason: (a && a.error) ? 'arm failed: ' + a.error : 'arm did not latch', concerns: v.concerns })
  } else {
    const reason = holdForAudit ? 'touches sq-qhy4 (pending external ZK audit) — maintainer decision'
      : !v.honest ? 'honest=false — held for maintainer'
      : !v.soundness_ok ? 'soundness_ok=false — held for maintainer'
      : (v.hold_reason || 'reviewer did not recommend arming')
    held.push({ pr: v.pr, reason, concerns: v.concerns })
  }
}
log('soundness lane: armed ' + armed.length + ', held ' + held.length)

return {
  reviewed: verdicts.length,
  armed,
  held,
  skipped: [],
}
