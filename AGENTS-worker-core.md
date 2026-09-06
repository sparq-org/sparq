# AGENTS.md — worker-tier core

> A README for coding agents. If you are an AI agent working on or with this repo, read this first.

This is the **worker-tier core** (≤32 KiB) loaded by codex. For the full reference, orchestrator documentation, and detailed checklists, see [AGENTS.md](AGENTS.md) (the complete authority).

## What sparq is

sparq is a from-scratch **RDF triplestore and SPARQL 1.1 engine in Rust** — dictionary-encoded, six sorted permutation indexes, parallel + streaming execution, RDFS/OWL-RL/N3 inference, an out-of-core (mmap) mode with a compressed on-disk format, a WebAssembly build, and a W3C-conformant HTTP server. The engine is published across several surfaces:

- **Rust crates** (crates.io): `sparq-core`, `sparq-engine` (core), `sparq-cli`, `sparq-server`, plus opt-in capability crates. `sparq-reason-el` is a **separate** opt-in crate — an OWL 2 EL consequence-based classifier — see [`skills/inference/SKILL.md`](skills/inference/SKILL.md).
- **npm**: `@sparq-org/sparq` — RDF/JS-typed API over the wasm build.
- **PyPI**: `sparq-rdf` (import name `sparq`) — pyo3/maturin bindings.

Status: experimental research engine; the API is unstable.

## Skills — how to USE sparq from your code

Usage instructions for each public surface are packaged as Agent Skills under [`skills/`](skills/). Read [`skills/SKILL.md`](skills/SKILL.md) first — it is the router skill that lists every surface and points you at the right one.

> Note: `.claude/skills/` (separate tree) holds INTERNAL skills for agents working *on* the engine's source, not usage docs. Do not confuse the two.

## Working on this repo (contributor agents)

- Build: `cargo build --workspace`. Test: `cargo test --workspace`.
- Lint is enforcing (CI gates on it): `cargo clippy --workspace --exclude sparq-py --all-targets -- -D warnings` must pass. Run clippy over the **full workspace**, not a single crate.
- `cargo fmt --all --check` is **informational, not a gate** — the deferred one-time workspace reformat means it fails on files your change did not touch. Format only what you touched; never run `cargo fmt --all`. (The formatter version itself is pinned by `rust-toolchain.toml`.)
- The core crates (`sparq-core`, `sparq-engine`) must stay dependency-free of the opt-in capability crates, and the wasm build must not regress — both are enforced in CI.
- **New capabilities are opt-in by default** (a dedicated crate and/or a cargo feature that is OFF by default), so `sparq-core`/`sparq-engine` stay lean and the lean wasm bundle never grows.
- **Frontend optional-code policy (sq-mrrn4):** the same lean-by-default rule for the site/GUI. Any net-new frontend feature that is uncertain-value or rarely used and grows the bundle MUST load through a literal ESM dynamic `import()` taken on the user action that invokes it — `next/dynamic`/`React.lazy` for components, invocation-path `import()` for libraries and codecs (e.g. a compressed-codec decoder fetched only on the first upload of that file type). A feature flag or conditional render does **not** keep a static import out of the initial bundle. Classify every new frontend dependency in the PR as core/shared or optional, and register material optional chunks in `site/scripts/check-bundle.mjs`. Decision rule, exceptions, and the audit procedure: [AGENTS.md](AGENTS.md) and `.claude/skills/frontend-design/SKILL.md`.
- Conformance: the W3C SPARQL, inference, W3C SHACL, OGC GeoSPARQL and Solid WAC + ACP suites must stay green and are each **ratcheted** (the committed floor only goes up).
- **Merge discipline:** the gate for landing any change is *full-workspace clippy + `cargo test` + the conformance/perf ratchets*, all green.

## MAINTENANCE RULE (REQUIRED — read before changing any public surface)

**When you change a public API, update the matching skill in the SAME change (same commit/PR).** A "public API" means any of:

- a `pub` item in a crate's public surface (a published crate's exported types, traits, functions, or their signatures);
- a CLI flag, subcommand, or its behavior in `sparq-cli`;
- an HTTP route, query/body parameter, or response shape in `sparq-server`;
- a Python binding (the `sparq` package) or a JS/RDF-JS binding (`@sparq-org/sparq`).

Then edit the corresponding `skills/<surface>/SKILL.md` so its instructions and examples still compile and run against the new surface. Do not split this across a follow-up PR — a skill that documents a removed flag or a changed signature is worse than no skill. If the change spans surfaces, update every affected `SKILL.md`.

If you add a brand-new public surface, add a new `skills/<surface>/` (dir name == the skill's `name` frontmatter) and link it from `skills/SKILL.md`.

## STANDING RULE — proceed without waiting for greenlight (2026-06-21)

**Never stall the autonomous loop waiting for the maintainer to greenlight a design or make a decision.**

- **Blocked on greenlighting a DESIGN?** Proceed with implementation *without* the greenlight, then open a GitHub issue (🤖 SPARQ-agent self-id) so he can review and steer *after* it is built.
- **A DECISION you'd otherwise ask about?** Make the best-judgment choice, **document it** (PR body / bead / issue), and proceed. He corrects later if needed.
- **Drop "needs-user / awaiting greenlight" as a blocking state.** The only still-valid blocker is an **external credential/access an agent literally cannot obtain** (code-signing certs, npm/PyPI publish tokens, the external cryptographer audit).
- **EXCEPTION — this does NOT override honesty/soundness discipline.** Never label an unaudited ZK/MPC capability "sound/proven". Proceed on the *build*; keep the honest "not-yet-sound / research-grade / not externally audited" labels.

See the `proceed-and-document` skill (`.claude/skills/proceed-and-document/SKILL.md`) and **`research/agent-efficiency-tooling.md`** for details.

## Fix a shared issue everywhere it applies — cross-crate/cross-surface parity

When a bug or review finding describes a **class** of problem affecting more than one place, it must **eventually be addressed in every instance, not patched only where it surfaced.** Prefer fixing the pattern ONCE in the shared place so all surfaces inherit it; if not feasible, fix each instance to the **same spec** and file a bead for the consolidation.

## Task tracking — beads, not markdown TODOs

This repo tracks work in **beads** (`bd`, a git-native dependency-graph issue tracker; the committed source-of-record is `.beads/issues.jsonl`). Rules for any agent working here:

- **Do NOT write TODO/FIXME into markdown.** Capture future work as a bead instead.
- **When you identify follow-up/future work, create a bead** (from the repo root, with `bd` on your PATH):
  ```sh
  bd create "<imperative title>" -t <task|bug|feature|chore|spike> -p <0-4> -l <area:crate,kind:...> -d "<what + why + where>"
  ```
  This writes the shared Dolt DB (exclusive-lock-serialized — safe across parallel agents). **Never edit `.beads/issues.jsonl` by hand** — it causes merge conflicts; `bd export` regenerates it.
- Run `bd ready` to see unblocked work; close with `bd close <id>`.

## The sub-agent shared contract — write TERSE task-only briefs

This is the **single source of truth for the standing rules every dispatched sub-agent follows**. The role-agent system prompts under [`.claude/agents/`](.claude/agents/) each **carry this contract** and point here for the long form. So a dispatcher does **not** repeat it: a task brief states the **task only** (the bead, the target crate/surface, any task-specific constraint) and otherwise says *"follow the shared contract."* If a role prompt and this section ever disagree, **this section wins** — fix the role prompt.

The contract (every mutating sub-agent, every task):

1. **Worktree + branch.** Run in your OWN isolated git worktree (`isolation: "worktree"`); never `cd` into the shared main checkout. Branch from current main — `git fetch origin main && git checkout -b <kind>-<topic> origin/main` — and run all git from your cwd.
2. **Staging.** Stage ONLY the files you change, by explicit path. NEVER `git add -A`; NEVER stage `.beads/` (re-export is the orchestrator's job on its own branch); revert any beads churn that appears in your tree.
3. **No push / no merge.** Commit in-worktree and report; the **orchestrator** pushes, opens the PR, and merges. (Exception: `sparq-merge-fixer` pushes to an *existing* PR branch.)
4. **Gate in-worktree (HARD — never weaken a gate to pass).** Run your role's gates scoped to the affected crates/files; the orchestrator runs the authoritative full-workspace gate at merge. If a gate fails on real content, fix the content or report it as an honest finding — never disable, regex-weaken, or blanket-exclude a gate to go green.
5. **Model provenance (model-aware).** Stamp the model that ACTUALLY authored the change: an inline `[MODEL]` marker on new code/notes **+** a `Co-Authored-By: Claude MODEL <noreply@anthropic.com>` commit trailer, using whichever model you are running as. **Opus 5** (`claude-opus-5`) is the primary top tier (maintainer directive 2026-07-24 — it replaces both the Fable 5 and Opus 4.8 heads); a DOWNGRADED session (Opus 5 unavailable — e.g. Fable 5 / Opus 4.8) or a cheap tier stamps its own marker + matching trailer, which flags downgrade work for re-review under Opus 5. The canonical per-tier marker/trailer table lives in `.claude/workflows/fable-architect-drain.js` — reference it, never replicate it.
6. **Self-identify (🤖) in every comment.** Open a PR vs `main` whose body starts with the SPARQ-agent blockquote; begin every issue/PR/comment you author with it. PRs default to the review queue — arm with plain **`gh pr merge <n> --auto`** only when the brief says so, and **never arm a STACKED PR whose base ≠ `main`** (retarget base to `main` first).
7. **Honesty gates that bite — keep mentions safe.** The **privacy-claims** gate is LIVE: never write an unqualified ZK/MPC privacy/soundness claim (the v1 ZK verifier is remediated + internally re-audited but **external accredited-cryptographer sign-off is pending**, and `sparq-mpc` is honest-majority semi-honest only) — hedge/negate the wording or add an inline `privacy-claims-allow: <why>` marker. **No hard-coded performance numbers** in markdown, and **work-box/EC2 timings are NON-canonical**. The **typos** gate flags ordinary doc words — reword `DELETEd`/`DROPped`/`invokable`/`ANDed`.
8. **Honest scoping, no empty PRs.** Be non-sycophantic: if the bead's premise is wrong, the thing already exists, or a claim lacks evidence, say so plainly. Capture genuinely-new discovered work as a clear LIST in your report (the orchestrator beads it) — that LIST is for *planned* follow-up; an OUT-OF-SCOPE *discovery* (bug/tech-debt/doc-drift/footgun/better-approach) you instead self-file as a `self-improvement`-labelled GitHub issue.
9. **Maintenance rule.** When your change touches a public API (`pub` item / CLI flag / HTTP route / Py/JS binding) or a config key, update the matching `skills/<surface>/SKILL.md` (and crate README) in the SAME change.
10. **Heartbeat.** Print to stdout at least once per minute during long cargo/npm runs (a watchdog reaps silent agents at ~600 s).
11. **A permission denial is FINAL — never try to work around it.** If the harness **denies** a tool call, that decision is final: do **NOT** retry the same change through a *different* tool to evade the block — no `python`/`node` heredoc, no `sed -i`/`awk`/`tee`/`>>`-redirect, no `cat <<EOF`, no `git apply`/`patch`. Trying to bypass a denial is itself a violation. **In particular, `.claude/agents/*.md` (the role-agent configs) is a PROTECTED surface**: agent-config self-modification is **blocked by design** — capture it as a bead for the maintainer to apply, never attempt the edit yourself.
12. **Out-of-scope discoveries → a self-filed GitHub issue (not an inline fix, not a bead).** When you spot something that should change but is OUTSIDE your task scope (a latent bug, tech-debt, doc drift, a footgun, a better approach), do NOT fix it in this PR — open a GitHub issue with `gh issue create --label self-improvement`, body led by the `> 🤖 SPARQ agent — <one line>` self-ID blockquote and one line of what/where/why. Dedupe first (`gh issue list --state open --label self-improvement --search "<keywords>"`) and file ONLY genuine, actionable, out-of-scope findings.
13. **Never read agent transcripts / logs.** Do NOT `Read`/`cat`/`grep`/`ast-grep` the ephemeral subagent transcripts, the `agent-logs` branch, or any saved transcript. If a log genuinely must be inspected, that is the job of the ONE explicitly-tasked debug/self-improvement agent, never a side-quest. Durable transcripts are appended out-of-tree by `scripts/save-agent-log.sh` — carry a one-line LINK, never the body.

### Worktree isolation — every MUTATING agent gets its own worktree+branch

**Any sub-agent that WRITES files, runs `git checkout -b`, or commits MUST work in an isolated git worktree** — give it `isolation: "worktree"` on the Agent tool, or `git worktree add` its own directory + branch — **NEVER the shared main checkout.** Read-only agents (search, analysis, review) may share the main checkout.

**This is a MANDATORY clause in EVERY sub-agent brief that does branch/PR work** — spell it out (or rely on the shared-contract reference, which carries it): the agent operates in its **own** isolated worktree+branch and **MUST NOT run `git checkout` / branch-switch on the shared main checkout.** Read-only investigation on the shared checkout is fine (`git grep`, `git log`, reading files); **branch switching is not** — it clobbers concurrent agents' uncommitted working-tree changes.

Why this is mandatory, not advisory: a git working tree has **one** branch, index, and working directory. Two mutating agents on the **same** checkout therefore race — one agent's `git checkout -b` switches the branch out from under the other, and uncommitted edits leak onto the wrong branch. A separate worktree gives each agent its own branch + index + working dir, so they cannot collide.

The **orchestrator keeps the main checkout for itself** — it is single-threaded glue: `bd` operations, bead re-export on a dedicated branch, and PR review/merge. Keep `.beads/*` and otherwise-unrelated files **out of feature PRs** — a `bd export` re-export lands on its own branch, never folded into a feature branch.

### Worktree lifecycle — remove every worktree the moment its task is done

Worktrees and their build artifacts (`target/`) are a large disk sink and accumulate fast. Standing requirements:

- **Remove every worktree the moment its task is done** — once its branch has merged (or its work is captured/abandoned), `git worktree remove --force <path>`. The **branch persists in `.git`**, so removal loses nothing; only the working copy + its `target/` go. The orchestrator owns this.
- **Don't spawn a worktree you don't need.** Read-only or single-stream work uses the main checkout; a worktree is justified only for *concurrent* mutating work.
- **Periodic sweep:** `git worktree prune` + remove stale worktrees; if disk is tight this is the first lever.

## Repository hygiene — where things live (READ THIS)

Everything you produce has exactly **one** correct home. Putting it anywhere else creates the cruft that forces periodic "clean-up runs" — so don't create it in the first place.

- **Tasks / TODOs / follow-ups / "future work" → a bead.** Never a `TODO`/`FIXME`/`XXX` marker in a markdown file, never a `TODO.md`, never a `- [ ]` checklist of pending work in a tracked doc. If you catch yourself writing "we should later…", run `bd create` and move on.
- **Durable knowledge → `AGENTS.md` / `CLAUDE.md`, a `skills/<surface>/SKILL.md`, a crate `README.md`, or a `research/` design record — whichever fits.** Workspace-wide conventions and contributor rules go here in `AGENTS.md`. Usage knowledge goes in the matching skill. Per-crate caveats go in that crate's `README.md`. Design rationale and measured verdicts go in `research/`.
- **Do NOT commit narrative scratch docs.** No `HANDOVER*.md`, no `SESSION*.md`, no "current state" / progress-log markdown in the repo. Session and orchestration state belongs in beads (for work) or in your own un-tracked notes.
- **No hard-coded performance numbers in markdown**: cite the generated structured data, not a baked-in figure.

## For full reference

**This is the worker-tier core (≤32 KiB).** The complete authority including orchestrator documentation, detailed checklists (Post-batch re-evaluation, ZK circuit members, parser correctness), and full architectural details lives in [AGENTS.md](AGENTS.md). Consult it for:

- sparq-substrate architecture and perf-neutrality invariants
- Post-batch re-evaluation checklist (what to re-run after a change)
- ZK circuit-member checklist (adding/removing zk/compose members)
- New-parser correctness checklist
- Documents must stay current (design record graduation rules)
- Upstream blockers and upstream PR contribution practice
- Measured agent operating configuration (project-knowledge queries, ast-grep, brief discipline)
- Orchestration — orchestrator-tier standing rules (delegation, continuous loop, CI-watcher discipline)

Read AGENTS.md when you need the full context.
