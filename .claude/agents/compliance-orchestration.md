---
name: compliance-orchestration
description: How the lead session orchestrates sparq's certification phase — parallel engineer agents per framework, parallel auditors, then consolidation + CDMC scoring. Read before launching the engineer/auditor waves.
metadata:
  type: lead-runbook
---

The certification phase is the natural place for **fan-out parallelism**: each framework's controls +
evidence are largely independent. This file is the lead's runbook for sparq's certification phase
(epic `sq-toze`, branch family `cert-<framework>`). Read alongside `compliance-engineer.md` +
`compliance-auditor.md`, and `research/production-certification-plan.md` (the framework set + rationale
+ the grounded gap register).

## Shared SPARQ contract

### Shared standing rules (all agents)
- **Out-of-scope discovery → a self-filed GitHub issue, NEVER an inline fix.** Spot a bug / tech-debt / doc drift / footgun / better approach that is outside THIS task? Do not fix it here — `gh issue create --label self-improvement` with a `> 🤖 SPARQ agent — <one line>` body and one line of what/where/why,. Dedupe first (`gh issue list --state open --label self-improvement --search "<keywords>"`); file ONLY genuine, actionable, out-of-scope findings, never a nit or style preference (SPAM guard). Issues = the git-native channel for *newly-discovered* work; beads = the *planned* task graph.

## Why parallel

Sequential would take many days for the 12-framework set (asvs, cis, sbom, ssdf, slsa, openssf,
memsafety, iso27001, cra, privacy, cryptoreview, cdmc — matching the `git worktree add` list below).
Each framework produces its own
`compliance/<framework>/` directory (control table + evidence + policy templates) — disjoint files,
disjoint thinking. The auditor pass per framework is independent. Only **consolidation + the final CDMC
re-score** is sequential. Keep the per-framework agents parallel and dispatch them with `run_in_background`.

## The shared-tree constraint (real — hit it in PSS)

Background agents in this environment all open the **same working directory**. Spawn N agents into one
checkout and they fight over branch checkout, `git add -A` sweeps each other's files, and their pushes
race each other. They
must be **physically isolated by worktree**.

The Agent tool's `isolation: "worktree"` param has historically failed at the harness level here
("Failed to resolve base branch HEAD"). Prefer **`git worktree add` from the lead** — the lead controls
the topology. (If `isolation: "worktree"` works in this session, it is equivalent; either is fine as
long as each agent has a disjoint tree + branch.)

```sh
# from main, after the conformance/fuzz/Miri lanes are green:
git worktree add ../sparq-asvs        -b cert-asvs
git worktree add ../sparq-cis         -b cert-cis
git worktree add ../sparq-sbom        -b cert-sbom
git worktree add ../sparq-ssdf        -b cert-ssdf
git worktree add ../sparq-slsa        -b cert-slsa
git worktree add ../sparq-openssf     -b cert-openssf
git worktree add ../sparq-memsafety   -b cert-memsafety
git worktree add ../sparq-iso27001    -b cert-iso27001
git worktree add ../sparq-cra         -b cert-cra
git worktree add ../sparq-privacy     -b cert-privacy     # GDPR + ISO 27701 + SOC2-privacy + data-flow/dpia
git worktree add ../sparq-cryptoreview -b cert-cryptoreview
git worktree add ../sparq-cdmc        -b cert-cdmc
```

Then spawn one engineer agent **per worktree**, `cwd` set to its worktree path. They run truly in
parallel — disjoint trees, branches, `compliance/<framework>/` folders, push targets. No `git checkout`
collisions, no `git add -A` sweeps (engineers stage explicit paths only).

## Stage gate before fan-out (the gap-fixes)

Many frameworks can only reach a **perfect score** once the cross-cutting gap-fix beads land
(`cargo-deny advisories PR-gate`, checked-in per-release SBOM + VEX, `.well-known/security.txt`,
OpenSSF Best-Practices badge, unsafe-justification register + cargo-geiger ratchet, CONTRIBUTING
secure-coding section, cargo-auditable/cargo-vet, the crypto-review doc). The dependency edges in the
bead graph (see `research/production-certification-plan.md`) encode this: a framework's
"perfect-score" assessment is **blocked** by its gap-fix beads. The lead may run gap-fix beads as their
own parallel wave **before** (or interleaved with) the assessment wave, since a framework auditor will
not sign off while its blocking gap is open.

## Wave 1 — engineers in parallel (one per framework)

Spawn all in a **single message** with multiple Agent tool calls (`run_in_background: true`). Each
brief:
- Inherits the `compliance-engineer` persona (read it).
- Works **only** in its assigned worktree dir + writes **only** under `compliance/<framework>/` (plus
  the shared `compliance/data-flow.md` / `dpia.md` / `threat-model.md` owned by the privacy engineer).
- Branch is pre-created (`cert-<framework>`); agent commits + pushes to it (explicit `git add` paths).
- Opens a **draft PR** against `main` titled `Certification readiness — <framework>`. Body starts
  `> 🤖 SPARQ agent`. References epic `sq-toze` + the framework's assessment bead.
- Honesty contract: every control labelled *implemented & verified* / *audit-ready* / *gap*; never
  overclaim; never contradict the ZK-not-sound verdict. CDMC is in this wave too (scores against the
  current branch state); if later work changes the codebase materially, the CDMC engineer re-scores.

## Wave 2 — auditors in parallel (one per framework)

Once each engineer's draft PR is up, spawn one `compliance-auditor` per PR, also parallel via separate
worktrees (or a fresh worktree on the engineer's branch — the auditor writes only under
`compliance/audit/`, no source). Each reports `FINDINGS: N` or `SIGN-OFF`.

## Iteration — engineer↔auditor per framework

For each framework: if the auditor returns findings, the lead re-engages that framework's engineer
(via `SendMessage` to the named teammate if the team model is up, else a fresh self-contained spawn
quoting the findings file) with the findings as input. Iterate until that auditor signs off with
**zero findings**. Different frameworks iterate at different speeds — fine, they're independent. The
target is a **perfect score on every framework** (auditor zero-findings sign-off), with the standing
caveat that external-auditor / external-cryptographer items remain external by definition.

## Consolidation (after all frameworks — including CDMC — sign off)

Lead merges all framework PRs into a single **integration PR** (`Certification readiness — full set`)
so the cert-ready state lives on one branch ready for merge to `main`. Write/finalize
`compliance/README.md` (index) + `compliance/gap-register.md` (consolidated cross-framework view). If
audit findings drove material `crates/` changes, **re-run the CDMC engineer→auditor pass once more**
against the consolidated state (CDMC scoring depends on codebase state more than the doc-heavy
frameworks). Close the gap-fix + assessment beads as they land on `main`; do **not** close epic
`sq-toze` until the consolidated integration PR merges and every framework has a sign-off.

## Team topology (use `TeamCreate` + `SendMessage` if available)

When `TeamCreate` + `SendMessage` are in the deferred-tool registry, prefer the team model:
1. `TeamCreate { team_name: "sparq-cert" }`.
2. Per framework, `Agent { team_name: "sparq-cert", name: "<fw>-engineer", subagent_type:
   "compliance-engineer", prompt: <self-contained brief with the worktree path + bead id> }`. Spawn all
   in one message.
3. Same for auditors when Wave 1 is done: `<fw>-auditor`.
4. On findings, `SendMessage { to: "<fw>-engineer", … }` — the engineer wakes with full prior context.
5. Graceful shutdown when all frameworks sign off.
Plain-text output is not visible to teammates — cross-agent messages go via `SendMessage`. Teammates
auto-idle after each turn; an idle teammate still accepts messages and wakes. If these tools aren't in
the registry, fall back to spawn-and-wait-final (each spawn fully self-contained, prior report quoted).

## Coordination guardrails

- Each engineer brief MUST contain the **upstream stop-gate** (no `gh pr create` against a non-owned
  repo). Engineers stage explicit paths only — never `git add -A` (burned in PSS).
- Each agent **prints a heartbeat to stdout at least once per minute** during long commands — the stall
  watchdog kills agents after ~600s of silent stdout (burned before).
- Engineers DO NOT touch `crates/` unless closing a real technical gap; if they do, green gates
  (`fmt`, `clippy -D warnings`, `cargo test --workspace`, plus Miri/fuzz for any `sparq-core` unsafe)
  before commit. No weakening to pass.
- Auditors NEVER edit source or the engineer's evidence; they write only under `compliance/audit/`.
- Subagents capture discovered work as `bd` beads (cd main checkout + `bd create`, reference `sq-toze`);
  never hand-edit `.beads/`. Orchestrator re-exports.
- Lead does NOT run worktrees in the same checkout as a running conformance/build agent — wait for that
  to merge first, then fan out. Identify every PR/issue/comment as **SPARQ agent** 🤖.
- NON-CANONICAL timing: this session runs on an EC2 work box; never bake measured timings into docs.

## Reporting back to the maintainer

After Wave 1: per-framework status table (controls drafted, evidence produced, anything that required a
`crates/` change). After each Wave-2 round: per-framework findings count + severity, trending to zero.
After consolidation: the **CDMC scorecard** (per-capability maturity + recommendations) as the
headline, plus the consolidated cert-readiness summary + the residual external-auditor/cryptographer
items.
