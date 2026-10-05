// Fable-architect drain (Fable collaboration tier — umbrella #1111 "revisit-with-fable"). [OPUS-4.8]
//
// PURPOSE: the flagship operating model where the EXPENSIVE stronger model (Claude Fable)
// acts as ARCHITECT + adversarial REVIEWER and a CHEAP fleet (Sonnet 4.6 / Haiku 4.5) does
// the mechanical middle. Fable touches an epic exactly TWICE — Stage1 (one decompose call
// per epic) and Stage5 (review of ESCALATED diffs only). Everything in between is cheap.
//
//   Stage1  Decompose   FABLE  sparq-architect: epic -> design record + N DISJOINT beads
//   Stage2  Recon       HAIKU  ground each bead's spec against the real tree (findings only)
//   Stage3  Implement   SONNET fleet fan-out, one isolated worktree per DISJOINT bead
//   Stage4  Verify      HAIKU  mechanical verify: ARM clean low-risk / ESCALATE the rest
//   Stage5  Review      FABLE  adversarial review of escalated diffs -> arm-on-verdict;
//                              disposition 'fable_implements' -> scoped Fable impl -> re-Stage4
//
// The beads Stage1 emits MUST have DISJOINT file/crate surfaces (curated disjoint-crate wave
// pattern) so the Stage3 fleet runs in parallel worktrees with ZERO merge conflicts.
//
// MODEL TIERS: opts.model carries the DISPATCH model taken from the TIER table below via
// dispatchModel(token) — NEVER a hand-written literal. Top-tier calls (token 'fable'/'opus')
// dispatch the FULL primary id `claude-opus-5`: the bare `fable`/`opus` harness ALIASES still
// lag onto the downgrade tiers (claude-fable-5 / claude-opus-4-8 — alias lag probe-proven in
// PR #3763), so an alias dispatch would silently serve a downgrade model while the table
// stamps [OPUS-5]. Cheap-tier calls keep their aliases (no rollout lag in play). opts.agentType
// names the role agents ('sparq-architect' for the decompose call, 'sparq-reviewer'
// for the review call), now APPLIED under .claude/agents/ (maintainer-authorized, commit
// c28d90e4); the agentType refs resolve to those role configs and opts.model overrides the tier
// on top of them. (If an agent is ever removed, an unknown agentType falls back to the default
// toolset and the model override alone still drives it.) Impl fan-out routes through
// pickAgent(surface) to the EXISTING implementer agents, just dropped to the Sonnet tier.
//
// DURABLE: committed under .claude/workflows/. Re-run with:
//   Workflow({ name: "fable-architect-drain", args: { epics: ["sq-...", "sq-..."] } })
//   Workflow({ name: "fable-architect-drain", args: { epic: "sq-..." } })
//
// SAFETY: impl agents (Sonnet fleet + any Stage5 Fable fix) run in ISOLATED worktrees, never
// branch-switching the shared /home/ubuntu/sparq checkout. Stage2 recon, Stage4 mech-verify,
// and Stage5 review are READ-ONLY (no worktree). Arming iterates VERDICT OBJECTS, never a
// blind PR-number loop (arm-gate-on-verdict rule). Judgment on decision-only beads is
// DELEGATED to the `proceed-and-document` skill so a hand-dispatched agent inherits it.

export const meta = {
  name: 'fable-architect-drain',
  description: 'Fable-tier flagship: FABLE architect decomposes an epic into disjoint beads (1 call/epic), a HAIKU/SONNET fleet grounds + implements + mechanically verifies them, and FABLE reviews only the escalated diffs (arm-on-verdict). Keeps the expensive model off the mechanical middle.',
  whenToUse: 'Run to drain an epic through the Fable collaboration tier: expensive model architects + reviews, cheap fleet does the disjoint implementation work.',
  phases: [
    { title: 'Guard', detail: 'per-epic disk guard before the worktree fleet' },
    { title: 'Decompose', detail: 'FABLE architect: epic -> design record + N disjoint beads' },
    { title: 'Recon', detail: 'HAIKU grounds each bead spec against the real tree (findings only)' },
    { title: 'Implement', detail: 'SONNET fleet: one isolated worktree per disjoint bead -> opt-in PR' },
    { title: 'Verify', detail: 'HAIKU mechanical verify: arm clean low-risk, escalate the rest' },
    { title: 'Review', detail: 'FABLE reviews escalated diffs only -> verdict -> arm-on-verdict' },
    { title: 'Arm', detail: 'iterate verdict objects; arm honest+recommend_arm; hold the rest' },
  ],
}

// CANONICAL per-tier marker/trailer table — the single source of truth (AGENTS.md § The
// sub-agent shared contract item 5: agent briefs POINT here and never replicate these
// mappings). Opus 5 (claude-opus-5) is the PRIMARY top tier (maintainer directive
// 2026-07-24), replacing both the Fable 5 and Opus 4.8 heads — the `fable`/`opus` keys are
// STABLE routing tokens (bead labels / dispatch params; do not rename) whose dispatch
// target is now Opus 5. The marker/trailer always follows the model that ACTUALLY served
// the run (harness --model; verify with scripts/fable/detect-tier.sh): a DOWNGRADED
// session (Opus 5 unavailable — e.g. Fable 5 / Opus 4.8) stamps its own row below, which
// flags the work for re-review under Opus 5. Existing [FABLE]/[FABLE-5]/[OPUS-4.8] stamps
// in history are accurate HISTORY — never rewrite them.
const TIER = {
  // Top-tier routing tokens — both dispatch the Opus 5 primary BY FULL ID. Full-ID dispatch
  // serves exactly the requested model (probe-proven in this PR: modelUsage billed
  // claude-opus-5 with no silent fallback), so on these rows dispatch-requested == served;
  // the bare `fable`/`opus` ALIASES still lag onto the downgrade tiers, which is why the
  // dispatch sites take `model` from THIS table (dispatchModel) and never pass the alias.
  fable:  { model: 'claude-opus-5', marker: '[OPUS-5]',     trailer: 'Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>' },
  opus:   { model: 'claude-opus-5', marker: '[OPUS-5]',     trailer: 'Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>' },
  // Cheap fleet tiers — dispatched by alias (no rollout lag in play; the aliases track the
  // current cheap models claude-sonnet-4-6 / claude-haiku-4-5, per orchestration/routing.toml).
  sonnet: { model: 'sonnet', marker: '[SONNET-4.6]', trailer: 'Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>' },
  haiku:  { model: 'haiku',  marker: '[HAIKU-4.5]',  trailer: 'Co-Authored-By: Claude Haiku 4.5 <noreply@anthropic.com>' },
  // ATTRIBUTION-ONLY rows (`model: null`). Fable 5 and Opus 4.8 were DEPRECATED as routing
  // targets on 2026-07-26 (maintainer: "deprecate the use of fable and opus entirely in favour of
  // opus5"), so neither is dispatchable any more — `model` is null and dispatchModel() THROWS on
  // these rows rather than returning undefined, which the harness would silently read as "use the
  // default model". They survive because attribution is not routing: scripts/fable/detect-tier.sh
  // can still detect that a run was SERVED by a downgraded tier, and the correct stamp for that
  // run is this row's marker/trailer. Existing [FABLE-5]/[OPUS-4.8] stamps are accurate HISTORY.
  'fable-5':  { model: null, marker: '[FABLE-5]',  trailer: 'Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>' },
  'opus-4-8': { model: null, marker: '[OPUS-4.8]', trailer: 'Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>' },
}
const mark = (t) => (TIER[t] || TIER.opus).marker
const trailer = (t) => (TIER[t] || TIER.opus).trailer
// The ONLY place a dispatch site may take opts.model from — marker/trailer/model all derive
// from the SAME selected row, so attribution and the live dispatches cannot diverge again.
// This workflow has no in-band served-model hook, so attribution rides the dispatched row:
// safe because full-ID dispatch == served (probe above); verify post-hoc with
// scripts/fable/detect-tier.sh, whose downgrade verdict routes stamping to the downgrade rows.
// FAILS CLOSED on a non-dispatchable tier. Returning `undefined` here would hand the harness no
// --model flag, i.e. a SILENT fallback to whatever the session default happens to be — the exact
// failure mode the 2026-07-26 deprecation is meant to remove. Refuse instead.
const dispatchModel = (t) => {
  const row = TIER[t] || TIER.opus
  if (!row.model) {
    throw new Error(
      `[OPUS-5] tier '${t}' is attribution-only and has no dispatchable model (Fable 5 / Opus 4.8 ` +
      `were deprecated as routing targets 2026-07-26 in favour of opus5). Dispatch 'opus'/'fable' ` +
      `(both -> claude-opus-5) instead; this row exists only to STAMP a detected downgraded run.`)
  }
  return row.model
}

const DISK_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    disk_state: { type: 'string', enum: ['OK', 'WARN', 'CRITICAL', 'UNKNOWN'] },
    disk_free_gb: { type: 'number' },
  },
  required: ['disk_state'],
}

const DECOMPOSE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    epic: { type: 'string' },
    design_record_path: { type: 'string' },
    design_record_pr: { type: 'string' },
    beads: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: {
          id: { type: 'string' },
          surface: { type: 'string' },
          title: { type: 'string' },
          spec: { type: 'string' },
          disjoint_surface: { type: 'string' },
        },
        required: ['id', 'surface', 'title'],
      },
    },
    notes: { type: 'string' },
  },
  required: ['epic', 'beads'],
}

const GROUND_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    id: { type: 'string' },
    ready: { type: 'boolean' },
    ground_refs: { type: 'array', items: { type: 'string' } },
    feature_flags: { type: 'array', items: { type: 'string' } },
    gaps: { type: 'array', items: { type: 'string' } },
  },
  required: ['id', 'ready'],
}

const IMPL_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    bead: { type: 'string' },
    pr_url: { type: 'string' },
    gates_green: { type: 'boolean' },
    skipped: { type: 'boolean' },
    reason: { type: 'string' },
  },
  required: ['bead', 'skipped'],
}

// Haiku mechanical verdict: it ARMS the clean, low-risk, purely-mechanical passes itself
// (it has Bash) and ESCALATES anything with judgment/soundness content up to Fable.
const MECH_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    bead: { type: 'string' },
    pr: { type: 'string' },
    armed: { type: 'boolean' },
    escalate: { type: 'boolean' },
    honest: { type: 'boolean' },
    reason: { type: 'string' },
  },
  required: ['bead', 'escalate'],
}

// Fable reviewer verdict (READ-ONLY): the orchestrator arms from these objects. disposition
// 'arm' -> arm; 'hold' -> leave OPEN for the maintainer; 'fable_implements' -> Fable fixes it.
const FABLE_VERDICT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    honest: { type: 'boolean' },
    recommend_arm: { type: 'boolean' },
    disposition: { type: 'string', enum: ['arm', 'hold', 'fable_implements'] },
    concerns: { type: 'array', items: { type: 'string' } },
  },
  required: ['honest', 'recommend_arm', 'disposition'],
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

// Route a disjoint bead's surface to the EXISTING implementer agent (dropped to Sonnet tier).
function pickAgent(surface) {
  const s = (surface || '').toLowerCase()
  if (s.includes('site') || s.includes('gui')) return 'sparq-site'
  if (s.includes('ci') || s.includes('infra') || s.includes('release') || s.includes('workflow')) return 'sparq-ci-infra'
  if (s.includes('doc')) return 'sparq-docs'
  if (s.includes('sparq-') || s.includes('engine') || s.includes('serve') || s.includes('crate') || s.includes('zk') || s.includes('mpc') || s.includes('shacl') || s.includes('vectors') || s.includes('reason')) return 'sparq-rust-feature'
  return 'general-purpose'
}

function diskGuardPrompt() {
  // [OPUS-4.8] sq-4vo9m: same per-tick disk guard the autonomous scheduler runs — the fleet's
  // per-agent target/ dirs re-fill the disk; an unguarded loop previously hit 99%/ENOSPC.
  return 'Run the disk guard (READ-ONLY repo; do NOT git-checkout): ' +
    '`cd /home/ubuntu/sparq && bash scripts/disk-guard.sh --apply --reclaim-main-target`. ' +
    'Capture its final `[disk-guard] done (state=..., advisory exit=N)` line and report ' +
    'disk_state (OK/WARN/CRITICAL/UNKNOWN) and disk_free_gb (the "Xg free" it printed, as a number).'
}

function architectPrompt(epic) {
  const t = 'fable'
  return 'You are the FABLE ARCHITECT for the Fable collaboration tier (umbrella #1111 "revisit-with-fable"). ' +
    'Decompose epic ' + epic + ' of `sparq-org/sparq` into (a) a design record under `research/` and (b) N DISJOINT implementation beads a cheap Sonnet fleet can build in PARALLEL. ' +
    '`export PATH=$PATH:/home/ubuntu/.local/bin && bd show ' + epic + '` and read the linked design records / code first. ' +
    'HARD constraint — the beads MUST have NON-OVERLAPPING file/crate surfaces (curated disjoint-crate wave pattern): at most one bead per crate/module so parallel worktrees NEVER conflict. If the epic cannot be cut into disjoint pieces, say so in `notes` and emit fewer beads. ' +
    'For each bead give a crisp `spec` (what to build + acceptance criteria) and a `disjoint_surface` (the exact files/crate it owns). New capability => opt-in/feature-gated (keep sparq-core/engine lean). ' +
    'CREATE each bead with `bd create` (cd the main repo; do NOT edit .beads/ files by hand) and return its real id. ' +
    'You MAY open ONE design-record PR (synthesis/finalize role — reserve PR-opening for synthesis per the researcher-PR gotcha; the fleet, not you, implements). 🤖 SPARQ self-id blockquote in the PR body; tag ' + mark(t) + ' + the trailer `' + trailer(t) + '`. ' +
    'HONESTY (non-sycophantic): distinguish implemented vs designed vs proposed; NO fabricated numbers; keep any ZK/MPC framing caveated (v1 verifier internally re-audited, EXTERNAL sign-off PENDING sq-qhy4, MPC semi-honest-only); no hard-coded perf numbers; work-box timings NON-canonical. ' +
    'Return {epic, design_record_path, design_record_pr, beads:[{id,surface,title,spec,disjoint_surface}], notes}. If the epic is not decomposable into disjoint beads, return {epic, beads:[], notes:"..."}.'
}

function reconPrompt(b, decomp) {
  return 'GROUND the spec for bead ' + b.id + ' ("' + b.title + '", surface ' + b.surface + ') against the REAL tree so the implementer does not rediscover it. ' +
    'Spec from the Fable architect: ' + JSON.stringify(b.spec || '') + '. Design record: ' + JSON.stringify((decomp && decomp.design_record_path) || '') + '. ' +
    'FINDINGS ONLY — do NOT open a PR and do NOT git-checkout the shared tree (Explore-style). Use Read/Grep/Glob to locate: the exact files/anchors to edit, the existing pattern to mirror, the crate + any feature flags to gate BOTH states, and where the direct unit test belongs (coverage ratchet is a per-crate LINE floor — one DIRECT test per new public fn). ' +
    'Report READY=true only if a Sonnet implementer has everything it needs; otherwise READY=false and list the concrete gaps. ' +
    'Return {id, ready, ground_refs:[file:anchor ...], feature_flags:[...], gaps:[...]}.'
}

function implPrompt(b, ground, tier) {
  return 'Implement disjoint bead ' + b.id + ' ("' + b.title + '", surface ' + b.surface + ') on the ' + tier.toUpperCase() + ' tier. ' +
    '`export PATH=$PATH:/home/ubuntu/.local/bin && bd show ' + b.id + '`. You are in an ISOLATED git worktree — work there; NEVER git-checkout/branch-switch the shared /home/ubuntu/sparq checkout. Stay INSIDE your `disjoint_surface` (' + JSON.stringify(b.disjoint_surface || b.surface) + ') so you do not collide with a sibling fleet agent. ' +
    'Grounding from recon (use it; do not re-derive): ' + JSON.stringify(ground || {}) + '. ' +
    'FIRST decide if genuinely implementable autonomously: if it needs an EXTERNAL credential/access you cannot obtain (OS signing certs, npm/PyPI tokens, an external auditor), return {bead, skipped:true, reason}. If it merely needs a DESIGN/JUDGMENT decision or is underspecified, PROCEED per the `proceed-and-document` skill (.claude/skills/proceed-and-document/SKILL.md): best-judgment choice, DOCUMENT it in the PR body + a one-line bead note, open a short 🤖-self-id GitHub issue for the maintainer to steer later. ' +
    'Implement per the bead + AGENTS.md: opt-in/feature-gated; gate BOTH feature states (clippy --all-targets --all-features -D warnings + tests in default AND feature-on), rustdoc --all-features -D warnings (avoid an intra-doc link to a cfg-gated item — use a code span), cargo fmt, check-readme-template.py if you touch a crate README (<=120 lines + emoji + License), check-privacy-claims.sh clean, typos gate, no hard-coded perf numbers, add ONE DIRECT unit test per new public fn (coverage ratchet). ' +
    'Tag ' + mark(tier) + ' + the trailer `' + trailer(tier) + '`. Explicit-path staging (no `git add -A`, never `.beads/`). Close the bead on PR-open. Open ONE PR vs main, auto-merge OFF, with the 🤖 SPARQ-agent self-id blockquote. ' +
    'Return {bead, pr_url, gates_green, skipped:false}. If you hit a weekly limit, STOP and return {bead, skipped:true, reason:"weekly limit"}.'
}

function mechPrompt(impl, b) {
  return 'MECHANICAL verify PR ' + impl.pr_url + ' (bead ' + b.id + ', surface ' + b.surface + ') on the CHEAP tier. READ-ONLY: `gh pr diff` + the changed files (do NOT git-checkout the shared tree). ' +
    'You do the DETERMINISTIC checks only: (1) CI gates green in BOTH feature states; (2) the diff stays within the bead scope + its disjoint surface; (3) tests are real, not vacuous; (4) explicit-path staging, no `.beads/` in the PR; (5) markers/trailer present. ' +
    'ARM only a purely-mechanical, clean, LOW-RISK pass: `gh pr merge ' + impl.pr_url + ' --squash --auto`, then return {bead, pr, armed:true, escalate:false, honest:true}. ' +
    'ESCALATE to Fable (armed:false, escalate:true) — do NOT arm — if the diff touches ANY judgment surface: an honesty/soundness claim, a ZK/MPC/security-sensitive change, a public-facing irreversible change, a perf claim/number, a non-obvious algorithm, or anything you are not confident is purely mechanical. When unsure, ESCALATE (fail toward Fable review). ' +
    'Return {bead:"' + b.id + '", pr:"' + impl.pr_url + '", armed, escalate, honest, reason}.'
}

function fableReviewPrompt(r) {
  return 'You are the FABLE REVIEWER. A cheap mechanical pass ESCALATED PR ' + r.pr + ' (bead ' + r.bead + ') because it carries a judgment/soundness question. ' +
    'READ-ONLY adversarial review: `gh pr diff ' + r.pr + '` + `gh pr view ' + r.pr + '` + the changed files (do NOT git-checkout, do NOT push, do NOT merge — you return a VERDICT the orchestrator acts on). ' +
    'Judge: is it HONEST (no overclaim; any ZK/MPC claim stays "research-grade / not externally audited — sq-qhy4"; MPC semi-honest-only; no hard-coded perf; work-box timings non-canonical), SOUND, correctly scoped, and does it actually satisfy the bead? ' +
    'Choose a `disposition`: "arm" (clean + low-risk -> recommend_arm:true); "hold" (leave OPEN for the maintainer — any honesty/soundness concern, a security/ZK-soundness-sensitive surface, a public-facing irreversible change, or low confidence); "fable_implements" (the diff is close but the correct fix is subtle enough that YOU should implement it directly, in a scoped isolated worktree, rather than bounce it back to the cheap fleet). ' +
    'Non-sycophantic: never rubber-stamp, and never invent a concern the diff does not support. ' +
    'Return {honest, recommend_arm, disposition, concerns:[...]}.'
}

function fableImplPrompt(v) {
  const t = 'fable'
  return 'You (FABLE) chose disposition "fable_implements" for bead ' + v.bead + ' (PR ' + v.pr + '). Take the wheel: in an ISOLATED git worktree (never branch-switch the shared checkout), implement the correct fix. ' +
    'Concerns to resolve: ' + JSON.stringify(v.concerns || []) + '. ' +
    'Either push a fix commit to that PR branch or open ONE corrected PR vs main (auto-merge OFF, 🤖 self-id blockquote). Full gate discipline (both feature states, rustdoc, fmt, privacy-claims, typos, one DIRECT test per new public fn, no hard-coded perf). Explicit-path staging, no `.beads/`. Tag ' + mark(t) + ' + trailer `' + trailer(t) + '`. ' +
    'Return {bead:"' + v.bead + '", pr_url, gates_green, skipped:false}. It will be re-checked by the cheap mechanical verify.'
}

function armPrompt(v) {
  return 'Arm PR ' + v.pr + ' (bead ' + v.bead + ') for the merge train: the Fable reviewer returned disposition "arm" with honest=true and recommend_arm=true. ' +
    'Run `gh pr merge ' + v.pr + ' --squash --auto` and confirm auto-merge latched. Do NOT force-merge; if the arm command errors, report it. ' +
    'Return {armed, pr:"' + v.pr + '", error}.'
}

async function mechVerify(impl, b) {
  if (!impl || impl.skipped || !impl.pr_url) {
    return { bead: b.id, surface: b.surface, pr: null, armed: false, escalate: false, honest: false, skipped: true, reason: (impl && impl.reason) || 'no PR produced' }
  }
  const m = await agent(mechPrompt(impl, b), { label: 'mech:' + b.id, phase: 'Verify', schema: MECH_SCHEMA, agentType: 'general-purpose', model: dispatchModel('haiku') })
  // [OPUS-4.8] Thread b.surface through the verdict object so the Stage5 'fable_implements'
  // path can route pickAgent(v.surface) to the right implementer agent (else it is undefined
  // and dispatch silently falls back to general-purpose).
  return { bead: b.id, surface: b.surface, pr: impl.pr_url, armed: (m && m.armed) || false, escalate: (m && m.escalate) || false, honest: (m && m.honest) || false, reason: (m && m.reason) || '' }
}

async function processEpic(epic) {
  // [OPUS-4.8] sq-4vo9m: disk guard BEFORE the expensive Fable decompose — a CRITICAL disk
  // means the worktree fleet cannot run this pass, so do not even pay for the architect call.
  phase('Guard')
  const disk = await agent(diskGuardPrompt(), { label: 'disk:' + epic, phase: 'Guard', schema: DISK_SCHEMA, agentType: 'general-purpose', model: dispatchModel('haiku') })
  const diskState = (disk && disk.disk_state) || 'UNKNOWN'
  log('disk: state=' + diskState + (disk && disk.disk_free_gb != null ? ' (~' + disk.disk_free_gb + 'G free)' : ''))
  if (diskState === 'CRITICAL') {
    log('disk CRITICAL — deferring epic ' + epic + ' (guard pruned/reclaimed; re-run next tick)')
    return { epic, design_record: null, beads: [], armed: [], open_for_review: [], skipped: [{ bead: epic, reason: 'disk CRITICAL — fleet deferred' }] }
  }

  phase('Decompose')
  const decomp = await agent(architectPrompt(epic), { label: 'architect:' + epic, phase: 'Decompose', schema: DECOMPOSE_SCHEMA, agentType: 'sparq-architect', model: dispatchModel('fable') })
  const beads = ((decomp && decomp.beads) || []).filter(b => b && b.id)
  if (!beads.length) {
    return { epic, design_record: (decomp && decomp.design_record_path) || null, beads: [], armed: [], open_for_review: [], skipped: [{ bead: epic, reason: (decomp && decomp.notes) || 'architect produced no disjoint beads' }] }
  }
  log('decomposed ' + epic + ' -> ' + beads.length + ' disjoint beads: ' + beads.map(b => b.id).join(', '))

  // Stage2 — HAIKU recon grounds each bead in parallel (findings only, no worktree, no PR).
  phase('Recon')
  const grounded = await parallel(beads, (b) => agent(reconPrompt(b, decomp), { label: 'recon:' + b.id, phase: 'Recon', schema: GROUND_SCHEMA, agentType: 'general-purpose', model: dispatchModel('haiku') }))
  const groundById = new Map()
  grounded.forEach((g, i) => groundById.set(beads[i].id, g))

  // Stage3 -> Stage4 — SONNET fleet impl (isolated worktree) chained to HAIKU mechanical
  // verify, one disjoint bead per pipeline lane.
  phase('Implement')
  const built = await pipeline(
    beads,
    (b) => agent(implPrompt(b, groundById.get(b.id), 'sonnet'), { label: 'impl:' + b.id, phase: 'Implement', schema: IMPL_SCHEMA, agentType: pickAgent(b.surface), model: dispatchModel('sonnet'), isolation: 'worktree' }),
    (impl, b) => mechVerify(impl, b)
  )

  const armed = built.filter(r => r.armed).map(r => r.bead)
  const escalated = built.filter(r => r.escalate && r.pr)
  const open_for_review = []
  const skipped = built.filter(r => r.skipped).map(r => ({ bead: r.bead, reason: r.reason }))
  log('mech-verify: armed ' + armed.length + ', escalated ' + escalated.length + ', skipped ' + skipped.length)

  // Stage5 — FABLE reviews the ESCALATED diffs ONLY (read-only verdicts, one call per PR).
  let verdicts = []
  if (escalated.length) {
    phase('Review')
    verdicts = await parallel(escalated, (r) =>
      agent(fableReviewPrompt(r), { label: 'fable:' + r.bead, phase: 'Review', schema: FABLE_VERDICT_SCHEMA, agentType: 'sparq-reviewer', model: dispatchModel('fable') })
        .then(v => ({ bead: r.bead, pr: r.pr, surface: r.surface, honest: (v && v.honest) || false, recommend_arm: (v && v.recommend_arm) || false, disposition: (v && v.disposition) || 'hold', concerns: (v && v.concerns) || [] }))
    )
  }

  // Arm-on-verdict: iterate the VERDICT OBJECTS (never a blind PR-number loop / CI-status loop).
  phase('Arm')
  for (const v of verdicts) {
    if (v.disposition === 'arm' && v.honest && v.recommend_arm) {
      const a = await agent(armPrompt(v), { label: 'arm:' + v.bead, phase: 'Arm', schema: ARM_SCHEMA, agentType: 'general-purpose', model: dispatchModel('haiku') })
      if (a && a.armed) { armed.push(v.bead); continue }
      open_for_review.push({ bead: v.bead, pr: v.pr, concerns: (a && a.error) ? ['arm failed: ' + a.error] : v.concerns })
    } else if (v.disposition === 'fable_implements') {
      // Fable takes the wheel: scoped isolated-worktree Fable impl, then RE-ENTER Stage4
      // (haiku mechanical verify). Bounded to a single Fable-fix pass to avoid recursion.
      // Route through pickAgent for an IMPLEMENTER toolset (Edit/Write) — the architect role
      // forbids implementation — while opts.model forces the Fable tier onto that role.
      phase('Implement')
      const fix = await agent(fableImplPrompt(v), { label: 'fable-impl:' + v.bead, phase: 'Implement', schema: IMPL_SCHEMA, agentType: pickAgent(v.surface), model: dispatchModel('fable'), isolation: 'worktree' })
      phase('Verify')
      const m = await mechVerify(fix, { id: v.bead, surface: v.surface || '', title: '' })
      if (m.armed) armed.push(v.bead)
      else open_for_review.push({ bead: v.bead, pr: (fix && fix.pr_url) || v.pr, concerns: ['fable re-impl still needs review'].concat(m.reason ? [m.reason] : []) })
    } else {
      open_for_review.push({ bead: v.bead, pr: v.pr, concerns: v.concerns })
    }
  }

  return { epic, design_record: (decomp && decomp.design_record_path) || null, beads: beads.map(b => b.id), armed, open_for_review, skipped }
}

const EPICS = (args && Array.isArray(args.epics)) ? args.epics
  : (args && args.epic) ? [args.epic]
  : []

if (!EPICS.length) {
  log('no epic(s) given — pass args.epic or args.epics (a Fable decompose is one call/epic, so the caller names the epic)')
  return { epics_processed: 0, decomposed: [], armed: [], open_for_review: [], skipped: [{ bead: null, reason: 'no epic supplied' }] }
}

const results = []
for (const epic of EPICS) {
  if (typeof budget !== 'undefined' && budget && budget.total && budget.remaining && budget.remaining() < 120000) {
    log('token budget nearly exhausted — stopping before epic ' + epic)
    break
  }
  log('=== epic ' + epic + ' ===')
  results.push(await processEpic(epic))
}

return {
  epics_processed: results.length,
  decomposed: results.map(r => ({ epic: r.epic, design_record: r.design_record, beads: r.beads })),
  armed: results.flatMap(r => r.armed),
  open_for_review: results.flatMap(r => r.open_for_review),
  skipped: results.flatMap(r => r.skipped),
}
