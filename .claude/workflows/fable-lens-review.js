// Fable lens-review lane (Fable collaboration tier — umbrella #1111 "revisit-with-fable"). [OPUS-4.8]
//
// PURPOSE: point a single REVIEW LENS (e.g. privacy-claims, unsafe-sites, perf-honesty,
// coverage-ratchet, feature-gating) across the repo and let the EXPENSIVE model adjudicate
// ONE subtle question about it, cheaply. The cheap fleet does the legwork — HAIKU enumerates
// the lens's targets, SONNET builds the claim/evidence cross-reference tables — so FABLE reads
// a tight table and answers the judgment call, then the fleet files any follow-ups as beads.
//
//   Stage1  Recon      HAIKU  enumerate the lens's targets across the tree (findings only)
//   Stage2  Cross-ref  SONNET build claim->evidence->status tables over those targets
//   Stage3  Adjudicate FABLE  answer the subtle question: {findings[], soundness_verdict, follow_up_beads[]}
//   Stage4  File       fleet  create the follow_up_beads via `bd create`
//
// MODEL TIERS: opts.model drives the tier TODAY — the top-tier Adjudicate call dispatches the
// FULL primary id via TOP_TIER_MODEL below (canonical TIER table: fable-architect-drain.js; the
// bare 'fable' alias still serves the claude-fable-5 downgrade tier — alias lag probe-proven,
// PR #3763), while the cheap stages keep their 'sonnet'/'haiku' aliases. opts.agentType
// names the role agent ('sparq-reviewer'), now APPLIED under .claude/agents/
// (maintainer-authorized, commit c28d90e4); the agentType ref resolves to that role config and
// opts.model overrides the tier on top of it. (If the agent is ever removed, an unknown
// agentType falls back to the default toolset and the model override alone still drives it.)
//
// DURABLE: committed under .claude/workflows/. Re-run with:
//   Workflow({ name: "fable-lens-review", args: { lens: "privacy-claims", question: "..." } })
//
// SAFETY: Stages 1-3 are READ-ONLY (findings only; the recon + cross-ref fan-out uses
// Explore/findings-only agents so they do NOT each open a research/ PR — the researcher-PR
// gotcha). Stage4 only runs `bd create` in the main repo (no code PR, never edits .beads/
// files by hand); the orchestrator re-exports beads afterward. The FABLE adjudication is a
// single call and returns a verdict — it does not merge or push anything.

export const meta = {
  name: 'fable-lens-review',
  description: 'Fable-tier lens review: HAIKU enumerates a review lens\'s targets, SONNET builds claim/evidence cross-ref tables, FABLE adjudicates one subtle soundness question over the table, and the fleet files any follow-up beads. Expensive model reads a tight table, not the whole tree.',
  whenToUse: 'Run to adjudicate a single subtle cross-cutting review question (a lens) with the expensive model while the cheap fleet does the enumeration + cross-referencing.',
  phases: [
    { title: 'Recon', detail: 'HAIKU enumerate the lens\'s targets across the tree (findings only)' },
    { title: 'Cross-ref', detail: 'SONNET build claim -> evidence -> status tables over the targets' },
    { title: 'Adjudicate', detail: 'FABLE answer the subtle question: findings, soundness_verdict, follow_up_beads' },
    { title: 'File', detail: 'fleet creates the follow_up_beads via bd create' },
  ],
}

const TARGETS_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    lens: { type: 'string' },
    targets: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: {
          ref: { type: 'string' },
          kind: { type: 'string' },
          note: { type: 'string' },
        },
        required: ['ref'],
      },
    },
  },
  required: ['targets'],
}

const XREF_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    lens: { type: 'string' },
    tables: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: {
          claim: { type: 'string' },
          evidence: { type: 'string' },
          status: { type: 'string', enum: ['supported', 'unsupported', 'partial', 'unknown'] },
          refs: { type: 'array', items: { type: 'string' } },
        },
        required: ['claim'],
      },
    },
  },
  required: ['tables'],
}

const ADJUDICATION_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    summary: { type: 'string' },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: {
          issue: { type: 'string' },
          severity: { type: 'string', enum: ['info', 'low', 'medium', 'high', 'critical'] },
          refs: { type: 'array', items: { type: 'string' } },
        },
        required: ['issue'],
      },
    },
    soundness_verdict: { type: 'string', enum: ['sound', 'unsound', 'needs_work', 'inconclusive'] },
    follow_up_beads: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: {
          title: { type: 'string' },
          surface: { type: 'string' },
          rationale: { type: 'string' },
          priority: { type: 'string' },
        },
        required: ['title', 'surface'],
      },
    },
  },
  required: ['findings', 'soundness_verdict', 'follow_up_beads'],
}

const BEAD_FILE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    created: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: {
          id: { type: 'string' },
          title: { type: 'string' },
        },
        required: ['title'],
      },
    },
    failed: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: {
          title: { type: 'string' },
          error: { type: 'string' },
        },
        required: ['title'],
      },
    },
  },
  required: ['created'],
}

const LENS = (args && args.lens) || null
const QUESTION = (args && args.question) || ''
const SCOPE = (args && args.scope) || ''

// [OPUS-4.8] The File stage runs on the HAIKU tier (dispatch below). Emit the CONCRETE,
// greppable provenance marker for that tier rather than a `[<MODEL>]` placeholder the
// bead-filing agent has to substitute — a literal placeholder is easy to copy verbatim,
// producing non-greppable provenance. Keep FILE_MODEL and FILE_MARKER in lock-step.
const FILE_MODEL = 'haiku'
const FILE_MARKER = '[HAIKU-4.5]'

// Top-tier DISPATCH id — the Opus 5 primary BY FULL ID (single-sourced canonical TIER
// table: .claude/workflows/fable-architect-drain.js). Full-ID dispatch serves exactly the
// requested model (probe-proven, PR #3763); the bare 'fable' alias would silently serve
// the claude-fable-5 downgrade tier, so it is never passed as opts.model here.
const TOP_TIER_MODEL = 'claude-opus-5'

if (!LENS) {
  log('no lens given — pass args.lens (e.g. "privacy-claims" | "unsafe-sites" | "perf-honesty" | "coverage-ratchet") and optionally args.question')
  return { lens: null, soundness_verdict: 'inconclusive', findings: [], follow_up_beads: [], created_beads: [] }
}

function reconPrompt() {
  return 'Enumerate the TARGETS of the "' + LENS + '" review lens across `sparq-org/sparq`' + (SCOPE ? ' (scope: ' + SCOPE + ')' : '') + '. ' +
    'FINDINGS ONLY — do NOT open a PR and do NOT git-checkout the shared tree (Explore-style). Use Read/Grep/Glob (and ast-grep for code SHAPE questions). ' +
    'A "target" is a concrete site this lens must inspect — e.g. for privacy-claims: every unqualified ZK/MPC/privacy assertion + the check-privacy-claims allowlist; for unsafe-sites: every `unsafe` block + its justification register entry; for perf-honesty: every perf number/claim in markdown + its evidence source; for coverage-ratchet: each crate near its line floor + its thin public wrappers. ' +
    'Return {lens:"' + LENS + '", targets:[{ref:"file:anchor or crate", kind, note}]}. Keep it complete but tight.'
}

function xrefPrompt(targets) {
  return 'Build the claim -> evidence -> status CROSS-REFERENCE tables for the "' + LENS + '" lens so Fable can adjudicate from a tight table instead of the whole tree. ' +
    'Targets enumerated by recon: ' + JSON.stringify(targets) + '. ' +
    'FINDINGS ONLY — do NOT open a PR and do NOT git-checkout the shared tree (Explore-style). For each target, state the CLAIM it makes (or the invariant it must satisfy), the EVIDENCE that does or does not back it (the test, the audit-doc line, the justification-register entry, the generated perf data), a STATUS (supported | unsupported | partial | unknown), and the exact refs. Be precise and non-sycophantic — mark an unbacked claim `unsupported`, not `partial`. ' +
    'Return {lens:"' + LENS + '", tables:[{claim, evidence, status, refs:[...]}]}.'
}

function adjudicatePrompt(tables) {
  const q = QUESTION || 'Is the "' + LENS + '" lens SOUND across the repo as cross-referenced — i.e. does every claim/invariant trace to real, sufficient evidence, and what must change if not?'
  return 'You are the FABLE ADJUDICATOR for the "' + LENS + '" review lens. Answer ONE subtle question from the cross-reference table (read the underlying refs only where the table is insufficient). ' +
    'QUESTION: ' + q + '. ' +
    'Cross-ref tables from the Sonnet stage: ' + JSON.stringify(tables) + '. ' +
    'READ-ONLY (you return a verdict; you do not merge, push, or open a PR). Judge non-sycophantically and honestly: any ZK/MPC claim must stay "research-grade / not externally audited" (v1 verifier internally re-audited, EXTERNAL sign-off PENDING sq-qhy4, MPC semi-honest-only); no hard-coded perf numbers; work-box timings NON-canonical; distinguish implemented vs designed vs proposed. ' +
    'Emit FINDINGS (each with severity + refs), a single soundness_verdict (sound | unsound | needs_work | inconclusive), and a set of FOLLOW_UP_BEADS (title + surface + rationale + priority) that the fleet should file to close any gap — one bead per disjoint fix. ' +
    'Return {summary, findings:[{issue,severity,refs}], soundness_verdict, follow_up_beads:[{title,surface,rationale,priority}]}.'
}

function fileBeadsPrompt(beads) {
  return 'File these Fable-adjudicated follow-up beads for `sparq-org/sparq` (review lens "' + LENS + '"): ' + JSON.stringify(beads) + '. ' +
    'For each, run `export PATH=$PATH:/home/ubuntu/.local/bin && cd /home/ubuntu/sparq && bd create` with the title, a body carrying the rationale + the 🤖 SPARQ-agent self-id + a `' + FILE_MARKER + ' via fable-lens-review` provenance note (the concrete marker for this stage, per AGENTS.md rule 5 Model-provenance), the surface as a label/tag, and the priority if given. Do NOT edit `.beads/` files by hand and do NOT open a code PR — `bd create` only (the orchestrator re-exports afterward). Skip a bead only if an obvious duplicate already exists (say so in `failed`). ' +
    'Return {created:[{id,title}], failed:[{title,error}]}.'
}

// Stage1 — HAIKU recon: enumerate the lens's targets (findings only, no PR).
phase('Recon')
const rec = await agent(reconPrompt(), { label: 'recon:' + LENS, phase: 'Recon', schema: TARGETS_SCHEMA, agentType: 'general-purpose', model: 'haiku' })
const targets = (rec && rec.targets) || []
if (!targets.length) {
  log('lens "' + LENS + '" enumerated no targets — nothing to adjudicate')
  return { lens: LENS, soundness_verdict: 'inconclusive', findings: [], follow_up_beads: [], created_beads: [] }
}
log('lens "' + LENS + '" -> ' + targets.length + ' target(s)')

// Stage2 — SONNET cross-ref tables (findings only, no PR).
phase('Cross-ref')
const xref = await agent(xrefPrompt(targets), { label: 'xref:' + LENS, phase: 'Cross-ref', schema: XREF_SCHEMA, agentType: 'general-purpose', model: 'sonnet' })
const tables = (xref && xref.tables) || []
log('cross-ref built ' + tables.length + ' row(s)')

// Stage3 — FABLE adjudicate the subtle question (the single Fable call).
phase('Adjudicate')
const adj = await agent(adjudicatePrompt(tables), { label: 'fable:' + LENS, phase: 'Adjudicate', schema: ADJUDICATION_SCHEMA, agentType: 'sparq-reviewer', model: TOP_TIER_MODEL })
const findings = (adj && adj.findings) || []
const verdict = (adj && adj.soundness_verdict) || 'inconclusive'
const followUps = (adj && adj.follow_up_beads) || []
log('fable verdict: ' + verdict + ' (' + findings.length + ' finding(s), ' + followUps.length + ' follow-up bead(s))')

// Stage4 — fleet files the follow-up beads via bd create (cheap tier, no PR).
let created = []
if (followUps.length) {
  phase('File')
  const filed = await agent(fileBeadsPrompt(followUps), { label: 'file-beads:' + LENS, phase: 'File', schema: BEAD_FILE_SCHEMA, agentType: 'general-purpose', model: FILE_MODEL })
  created = (filed && filed.created) || []
  log('filed ' + created.length + ' follow-up bead(s)')
}

return {
  lens: LENS,
  summary: (adj && adj.summary) || '',
  soundness_verdict: verdict,
  findings,
  follow_up_beads: followUps,
  created_beads: created,
}
