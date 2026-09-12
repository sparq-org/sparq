# Maintenance flow-on automation — research + design

> Status: research/design record for review (bead **sq-ncvq**, epic). `[OPUS-4.8]`
> Goal: stop "flow-on" maintenance tasks falling through the gaps — a new
> crate/module shipping without a benchmark, a feature change landing without
> the doc/SKILL.md update, a new public surface with no conformance/cert, a new
> ZK circuit member without a gate-count baseline. The user cares about the
> GOAL (staying on top of maintenance), not a specific mechanism, and invited
> pushback on the proposal.

## 0. TL;DR — the recommendation

**Adopt nothing wholesale; reuse what we already have; build a thin local layer.
Do NOT fork beads into a typed-rule engine.**

The honest landscape verdict (Part 1) is that **no single product solves both
halves** of this problem, and the two halves live in different planes:

1. **Catch the gap at MERGE TIME (proactive, highest leverage).** "If `crates/foo/**`
   changed, a matching benchmark / README / `SKILL.md` must change too" is a
   *diff-aware PR-policy* problem. This repo **already has the machinery** — a
   `ci-summary` aggregator that is the single required check, a SKILL.md
   frontmatter validator, four ratchets (SPARQL/inference/SHACL conformance,
   coverage line-%, test-presence), a deterministic perf ratchet, a ZK
   gate-count snapshot test, and two advisory custom-checks
   (`check-no-perf-numbers.py`, `check-readme-template.py`). The right move is to
   **add a handful of new diff-aware "definition-of-done" (DoD) checks in the
   same idiom** and wire them into `ci-summary` — NOT to bolt on Danger-JS as a
   new Node dependency. (Danger is the industry-standard tool for exactly this,
   and is the fallback if the Python-script idiom proves too clumsy; but adding a
   Node toolchain + Dangerfile to a Rust repo whose gates are already Python is
   net negative — see §1.4.)

2. **Generate the durable, un-gateable follow-on (reactive, lower leverage but
   necessary).** Some flow-on work genuinely *cannot* be gated at merge — e.g.
   "this new competitor-relevant engine feature needs a `gather` follow-up", or
   "this new bench suite needs a dashboard row" (the dashboard is regenerated
   out-of-band). For those, **a thin declarative rule table + a merge-triggered
   script that runs `bd create`**. There is no off-the-shelf product that writes
   to `bd`; the closest analogs (Jira Automation, Linear) are tracker-locked
   SaaS. So this half is necessarily ours — but it is ~50 lines, not a beads
   fork.

**Why not the user's original proposal (typed beads + close-time rule engine)?**
Two reasons, both empirical: (a) the highest-value catch is at *merge*, before
the gap exists — a reactive bead created *after* a feature bead closes still
lets the un-benchmarked crate land on `main`; (b) beads v1.0.5 has **no
close-time hook and no typed-rule engine** (verified — §1.3), so the proposal
means forking/extending beads, which is more maintenance burden than the thin
local layer it would replace. We keep the *spirit* of the proposal (declarative
rules → auto-created follow-on beads) but trigger it from CI on PR-merge, not
from a beads internal hook, and we do the gateable subset as a hard merge gate
instead of a reactive bead.

```text
                 ┌─────────────────────────── a change is proposed (PR) ───────────────────────────┐
                 │                                                                                   │
   ┌─────────────▼──────────────┐                                          ┌─────────────────────────▼─────────────┐
   │  PROACTIVE  (merge-time)    │   gate fails → fix before merge          │  REACTIVE  (merge-time, on success)    │
   │  diff-aware DoD checks      │   ───────────────────────────────►       │  flow-on rule table → bd create ...    │
   │  (new Python checks +       │                                          │  (durable, un-gateable follow-ups:     │
   │   existing ratchets/gates), │                                          │   gather, dashboard row, cert epic)    │
   │   aggregated by ci-summary  │                                          │   surfaced as ready beads              │
   └────────────────────────────┘                                          └────────────────────────────────────────┘
```

---

## 1. Part 1 — does a tool exist? adopt vs extend-beads vs build

Research method: web research on the DoD-automation / follow-on-generation
landscape, plus a direct capability probe of `bd` v1.0.5 (`bd --help` and every
relevant sub-help) and a full inventory of the repo's existing CI gate
machinery. Sources are cited inline.

### 1.1 The landscape, honestly

| Tool / pattern | Solves PR-time DoD gating? | Solves follow-on task generation? | Verdict for us |
|---|---|---|---|
| **Danger JS** (`danger.systems/js`) | **Yes** — canonical use: read `danger.git.modified_files`, `warn()`/`fail()` if a matching file wasn't touched. One Dangerfile, one consolidated PR comment, can be a required check. | Technically (Octokit `issues.create` is reachable), but it's PR-anchored, awkward on merge/close, and **cannot write to `bd`**. | **Fallback only.** Right shape, but adds a Node toolchain + Dangerfile to a repo whose gates are already Python and already aggregated by `ci-summary`. Use only if the Python-check idiom proves too clumsy for diff logic. |
| **dorny/paths-filter** + a required gate job | **Yes** for cheap "touch X ⇒ run/require Y". | No. | Useful primitive; but we already detect changed paths trivially with `git diff --name-only`. Worth knowing the **required-check + skipped-job trap** (a skipped required check blocks merge): our `ci-summary` already solves this by *dynamically polling* check-runs rather than marking each path-conditional job required. |
| **Mergify** | Partial — gates merges on PR *state* (checks/labels/approvals); weaker than Danger at rich "X changed but Y didn't" diff logic. | **No** native create-issue action (only PR-scoped: comment/label/merge); would dispatch our own workflow anyway. | **Skip.** It's a merge-queue product; redundant with `ci-summary` + branch protection, and doesn't do the follow-on half. |
| **GitHub-native**: `on: pull_request: [closed]` + `if: merged`, `actions/github-script` / `gh`, issue forms, CODEOWNERS, required checks | The substrate for our own gates. | **The substrate for our own follow-on layer** — `github-script` `issues.create`, or a `run: bd create …` step. | **Adopt the substrate.** This is exactly how we hook the follow-on layer. Caveat: actions taken with the default `GITHUB_TOKEN` don't trigger further workflows (loop guard) — use a PAT/app token only if a created item must itself kick off automation. |
| **Policy-as-code**: OPA/Conftest, Allstar, repolinter, Scorecard, Powerpipe | These evaluate *config-as-data* / *security posture*, not PR *diffs*. Allstar can file issues — but only for its fixed security policy set. repolinter is archived. | No (except Allstar's fixed-policy issues). | **Category error — skip.** These answer "is branch protection on? does SECURITY.md exist?", not "did this crate change ship a benchmark?". (We already run CodeQL/Scorecard for the security axis.) |
| **Jira Automation / Linear automation rules** | n/a (they're trackers) | **Yes — the closest commercial analog**: trigger ("issue → Done") → condition → action ("create issue/sub-tasks"). But tracker-locked; cannot target `bd`. | **Borrow the mental model** (trigger → condition → action rule table), not the product. |

**Bottom line:** there is no single tool to adopt that does both (a) diff-aware
DoD gating and (b) follow-on generation against `bd`. The realistic answer is a
**hybrid**, and the steer in the bead is confirmed.

### 1.2 …but we are not starting from zero — the existing machinery to reuse

This repo already runs a mature DoD-gate estate (full inventory in §4). The two
load-bearing facts:

- **`ci-summary / gate`** (`.github/workflows/ci-summary.yml`, bead sq-prg4) is
  the *single required status check* for branch protection. It cannot use
  cross-workflow `needs:`, so it **polls every check-run on the head commit** and
  passes iff none of the *gating* ones failed. It **self-adapts** to added/renamed
  jobs and **excludes any check whose name matches `\b(advisory|informational)\b`**
  (sq-wjth). This means: **any new gate we add just needs to be a check-run that
  fails on violation; `ci-summary` picks it up automatically.** No central
  check-name list to maintain — the explicit anti-goal.
- The gate idioms already in use that new DoD checks should mirror:
  - **Python custom-check scripts** that scan git-tracked files and exit non-zero
    on violation, with an `--advisory` mode (`scripts/check-*.py`).
  - **Committed-floor ratchets** reviewed in diffs that may only rise
    (`bench/coverage-floor.json`, `bench/coverage-presence.json`,
    `bench/perf-baseline.json`, and the conformance constants in `ci.yml`).
  - **Snapshot tests** that fail when a tracked artifact changes without its
    baseline (`crates/sparq-zk-compose/tests/gate_count_snapshot.json`).

So the proactive half is "add a few more checks in idioms we already operate",
not "introduce a new tool class".

### 1.3 Does `bd` itself support TYPES / dependency RULES / close-time HOOKS?

Probed `bd` v1.0.5 directly. Findings:

- **TYPES: yes, and custom types are supported.** Built-ins: `task, bug, feature,
  chore, epic, decision, spike, story, milestone`. Custom types via
  `bd config set types.custom "type1,type2,…"`. So the user's "task TYPES" idea
  is *natively available* — we can tag follow-on beads with a `flow-on` /
  `bench` / `cert` flavour (label or custom type) for queryability.
- **Dependency RULES: partial.** `bd dep` / `--deps type:id` supports typed
  dependency *edges* (`blocks`, `discovered-from`, `relates_to`, `waits-for`),
  and `bd gate` supports **async wait gates including `gh:run` / `gh:pr`** (a bead
  can wait for a GitHub workflow or PR merge) and cross-rig `bead` gates. But
  there is **no "when a bead of type X closes, auto-create beads Y,Z" rule
  engine.** That logic does not exist in beads.
- **Close-time HOOKS: no.** `bd hooks` installs **git** hooks (pre-commit /
  post-merge / pre-push / post-checkout / prepare-commit-msg) for beads
  bookkeeping — there is **no bead-lifecycle "on close" hook** to attach
  rule-derived follow-on creation to.
- Adjacent capabilities that *do* exist and are worth using: `bd swarm` (epic →
  DAG of children for parallel work), `bd gate gh:run`/`gh:pr` (wait on CI/PR),
  `bd lint` (template-section linting), `bd query`/`bd search` (for the
  drift-scanner to find gaps), fan-out gates (`--waits-for` / `--waits-for-gate
  all-children|any-children`).

**Conclusion:** beads gives us the *data model* (custom types, typed deps, gates,
swarm DAGs) but **not** the close-time rule engine the original proposal needs.
Implementing the proposal *inside* beads means a non-trivial fork (a new
lifecycle-hook subsystem + a rule config + an evaluator). That is exactly the
"heavy typed-rule-engine fork" the steer warns against.

### 1.4 Adopt vs extend-beads vs build vs upstream — the verdict

- **Adopt (Danger/Mergify/OPA/…):** **No**, for the gateable half — we already
  operate the equivalent machinery in Python + `ci-summary`, and adding Danger's
  Node toolchain is net-negative duplication. **No**, for the follow-on half —
  nothing writes to `bd`.
- **Extend beads (fork a typed close-time rule engine):** **No.** Not supported
  in v1.0.5; would be a real subsystem to build and carry; the highest-value
  catch is at merge anyway, which beads cannot see.
- **Upstream a small beads contribution:** **Now moot — upstream already shipped
  it.** When this design was written (against beads v1.0.5) the genuinely small,
  generally-useful upstream contribution would have been a **bead-lifecycle hook**
  (run a configured command on `bd close`, analogous to the existing git-hook
  subsystem) so *any* beads user could wire on-close automation without a fork —
  worth a low-priority upstream *proposal issue* per our roll-your-own-then-upstream
  norm, but never blocking and never needed (the CI merge-trigger covers our case).
  **Update (2026-06, bead `sq-ncvq.17`):** before filing we checked upstream and
  found this feature **already requested, implemented, and merged** — the canonical
  repo (`github.com/steveyegge/beads` now redirects to `gastownhall/beads`) carries
  issue [#1754 "Beads lifecycle hooks (post-create, post-close, post-update)"][bd1754]
  in state **CLOSED / COMPLETED**, and the on-close hook fires from `bd close` /
  `bd update --status closed` per merged PR #2754 (the `on_close` hook), with live
  test coverage (`TestUpdateCloseHookFiring`, cf. issues #3800 / #3802 / #3805) and a
  config subsystem exposing `on_close` / `on_update` / `on_complete` (cf. #3782).
  Filing a proposal issue now would duplicate an already-implemented feature on a
  ~480-open-issue tracker, so per our **honesty + repo-hygiene** norms — don't ship a
  no-op; don't add maintainer noise — we **do not file** it, and treat the upstream
  action as satisfied. We still **do not need** the hook ourselves. The custom
  `types` / `labels` we *will* use were already upstream too. [OPUS-4.8]
- **Build a thin sparq-local layer:** **Yes** — this is the recommendation:
  1. a handful of new **diff-aware DoD checks** (Python, the existing idiom),
     wired into `ci-summary`;
  2. one **declarative flow-on-rules config** (`scripts/flow-on-rules.toml`) +
     one **merge-triggered script** (`scripts/flow-on.py`) that reads the PR's
     changed paths/labels and runs `bd create` for the un-gateable follow-ups;
  3. one **reusable drift-scanner** (`scripts/drift-scan.py`) for the periodic
     full sweep (Part 3), runnable locally and in a scheduled CI lane.

---

## 2. Recommended mechanism — concrete design

### 2.1 Proactive DoD gates (the merge-time half)

Each is a small `scripts/check-*.py` that diffs the PR against the merge base
(`git diff --name-only "$(git merge-base origin/main HEAD)"...HEAD`), applies one
rule, and exits non-zero on violation (with an `--advisory` mode for soft
rollout, mirroring `check-no-perf-numbers.py`). They run as jobs in `ci.yml` /
`docs-quality.yml` and are picked up automatically by `ci-summary` (no
check-list edit needed). Soft-launch each as `advisory`, then flip to hard once
the back-catalogue is clean (the same path `check-skill-frontmatter.py` took).

| # | Gate (new check) | Trigger (diff predicate) | Required follow-on it enforces | Escape hatch |
|---|---|---|---|---|
| G1 | **new-crate-completeness** | a new `crates/<x>/Cargo.toml` appears | the crate must have a `README.md` (already template-checked) **and** either a registered bench in `bench/benchmarks.toml` (a `source = crates/<x>` or example-bench entry) **or** a `publish = false` stub-exempt marker; if it's a public surface (see surface map) also a `skills/<surface>/SKILL.md`. | `<!-- flow-on-exempt: reason -->` in the crate README, recorded as a bead. **Implemented (#5701)** in `gate-new-crate.py` (`crate_flow_on_exempt`), reused by `flow-on.py`; a non-empty reason is required. |
| G2 | **public-api→skill** | a `pub` change in a published crate's surface / a `sparq-cli` flag / `sparq-server` route / `sparq-py`/`sparq-wasm` binding (path-heuristic: changed files under the crate's public modules) | the matching `skills/<surface>/SKILL.md` is also in the diff | per-PR label `skill-not-needed` (justified in PR body) |
| G3 | **new-bench→registry+dashboard** | a new `bench/<suite>/` dir or a new `benchmarks.toml` id | the suite is registered in `bench/benchmarks.toml` **and** (if a capability suite) either added to `FEATURED_SUITES` in `bench/dashboard/dashboard.js` **or** flagged `featured = false` in its toml entry | `featured = false` toml flag |
| G4 | **new-unsafe→justification** | a net-new `unsafe` block/`unsafe fn` (diff adds `unsafe`) | a matching entry in an `unsafe-justifications.md` ledger (file + reason + invariant) **and** the crate is in the Miri lane allow-list (`miri.yml`) | none — unsafe always needs justification |
| G5 | **zk-circuit-member→gate-count** | a new member dir under `zk/compose/` **or** any new `bin`-type Noir circuit elsewhere under `zk/` | a baseline row in `crates/sparq-zk-compose/tests/gate_count_snapshot.json` (or an `exempt_circuits` entry for a non-deployed `bin` such as a test harness). **Implemented (sq-ncvq.8):** `snapshot_covers_every_member` enforces the `zk/compose/` family; `snapshot_covers_top_level_circuits` extends coverage to every top-level `zk/` proving circuit. Both run without the toolchain. | re-baseline via `bench/zk-compose/scripts/gate_counts.sh` |
| G6 | **new-config/flag→docs** | a new public config key / CLI flag / env var (`sparq-cli`, `sparq-server`) | the matching `SKILL.md` / crate README documents it (subset of G2, but covers config not just `pub`) | label `config-internal` |

Notes:
- G2/G6 overlap with the **existing AGENTS.md norm** (the public-API→SKILL.md
  rule, `AGENTS.md:42-53`, and the per-change re-evaluation table,
  `AGENTS.md:104-120`). Today those are *honor-system* prose; G2/G6 make them
  *enforced*. The AGENTS.md table stays the human-readable index and gets a
  column noting which row is now machine-enforced.
- G1/G3 are heuristic (path-based) and intentionally start **advisory** —
  false-positives are a nag comment, not a block, until tuned.
- G4 (unsafe ledger) is genuinely new policy; today only `cargo-geiger` reports
  unsafe *informationally* (excluded from the gate). G4 turns it into a tracked
  ledger + Miri coverage requirement.

### 2.2 The flow-on-rules config + thin automation (the reactive half)

For follow-ups that **cannot** be gated at merge (the artifact is produced
out-of-band, or the follow-up is research/competitive work), a declarative rule
table drives `bd create` on PR-merge.

**`scripts/flow-on-rules.toml`** (illustrative shape):

```toml
# Each rule: a trigger (changed-path glob and/or label and/or merged-PR title
# regex) → one or more follow-on bead templates. Evaluated by scripts/flow-on.py
# on `pull_request: [closed]` when merged==true. Idempotent: a rule is skipped
# if an OPEN bead already exists with the same dedup-key.

[[rule]]
id = "new-competitor-feature-gather"
when_paths = ["crates/sparq-engine/src/exec/**", "crates/sparq-core/src/store/**"]
when_label = "competitor-relevant"        # author opts in via label
create = [
  { type = "task", labels = ["bench","gather"], dedup_key = "gather-{pr}",
    title = "gather: refresh competitor baselines after {pr_title}",
    body  = "Merged PR #{pr} touched a competitor-relevant engine path; re-run scripts/gather-competitors.sh and update bench/competitors.json." },
]

[[rule]]
id = "new-bench-dashboard-row"
when_paths = ["bench/*/benchmarks.toml", "bench/*/run.sh"]   # heuristic: a new suite
create = [
  { type = "task", labels = ["docs","dashboard"], dedup_key = "dash-{suite}",
    title = "dashboard: add a FEATURED_SUITES row for {suite}",
    body  = "A new bench suite landed; add it to bench/dashboard/dashboard.js or flag featured=false." },
]

[[rule]]
id = "new-public-surface-cert"
when_paths = ["crates/*/Cargo.toml"]      # new crate (flow-on.py checks it's new + public)
create = [
  { type = "epic", labels = ["docs","cert"], dedup_key = "cert-{crate}",
    title = "cert: conformance/cert plan for new surface {crate}",
    body  = "New public surface {crate}; decide on a conformance/recall/cert ratchet and wire it (cf. sparq-shacl crate-local W3C ratchet)." },
]
```

**`scripts/flow-on.py`** (the ~50-line automation):
1. inputs: the merged PR number, its changed-file list (`gh pr diff --name-only`
   or the Actions event payload), its labels, its title;
2. for each rule, evaluate the trigger (path globs ∧/∨ label ∧/∨ title regex);
3. for each matched `create` template, expand `{pr}`/`{crate}`/`{suite}`/… and
   compute `dedup_key`; query `bd` (`bd list --json` / `bd search`) for an OPEN
   bead carrying that dedup-key label — skip if present (idempotency);
4. otherwise `bd create --type <t> --labels <…> --title <…> --description <…>
   --deps discovered-from:<the feature bead if known>` and emit the new id to the
   step summary.
- All created beads carry a `flow-on` label and the rule `id`, so they're
  queryable (`bd list --label flow-on`) and auditable.
- The orchestrator/agent re-exports `.beads/` after the run (never hand-edited),
  per the beads hygiene norm.

**Wiring** — `.github/workflows/flow-on.yml`:
```yaml
on:
  pull_request:
    types: [closed]
jobs:
  flow-on:
    if: github.event.pull_request.merged == true
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@<pinned-sha>
      - run: python scripts/flow-on.py --pr "${{ github.event.pull_request.number }}"
        env: { GH_TOKEN: "${{ secrets.GITHUB_TOKEN }}" }
```
(Name it without `advisory`/`informational` so it's visible; it is *not* a gate —
it never blocks — it only mints follow-on beads. If a created bead must itself
trigger further automation, swap to a PAT, per the GITHUB_TOKEN loop-guard.)

**CI-only guard (bead sq-z0se).** Both issue-minting tools — `flow-on.py` and the
periodic `drift-scan.py` — REFUSE to file GitHub issues unless a CI marker
(`GITHUB_ACTIONS` or `CI`) is set; their `main()` calls `require_ci()` on the
write path only. This exists because an accidental dev-box run of `drift-scan.py`
once minted 20 spurious issues (#397-416). `--dry-run` is *never* guarded (it
makes zero `gh` calls), so local previews and the hermetic test-suite are
unaffected; a deliberate manual mint can set `DRIFT_SCAN_ALLOW_LOCAL=1` /
`FLOW_ON_ALLOW_LOCAL=1` as an explicit escape hatch.

### 2.3 Why this split is the right altitude

- The **gates** stop the gap from ever landing on `main` — the highest leverage,
  and they cost nothing new conceptually (same idioms + free `ci-summary`
  pickup).
- The **rule table** captures only the *genuinely un-gateable* follow-ups, in
  ~one config file + one script — far less surface than a beads fork, and it
  keeps the *spirit* of the user's proposal (declarative rules → auto follow-on
  beads).
- The **AGENTS.md per-change table** remains the single human-readable index;
  each row is annotated "enforced-by: Gn" or "flow-on: rule-id" so prose and
  automation can't silently diverge (and the table's own "keep this in sync"
  note now has teeth).

---

## 3. Part 2 — the rule taxonomy (rules that SHOULD exist)

Every rule grounded in the repo. "Enforce" column: **G** = proactive merge gate
(§2.1), **F** = flow-on bead (§2.2), **N** = already an AGENTS.md norm (prose),
**E** = already enforced today.

| Rule | Trigger | Required follow-on(s) | Enforce |
|---|---|---|---|
| New crate/module → benchmark presence | new `crates/<x>/Cargo.toml` | registered bench in `bench/benchmarks.toml` (or `publish=false` exempt) | **G1** (new) |
| New crate/module → README (template) | new `crates/<x>/` | `README.md` passing `check-readme-template.py` | G1 + **E** (advisory `check-readme-template.py`; flip to hard) |
| New **public surface** → `SKILL.md` | new public crate/CLI/HTTP/Py/JS surface | `skills/<surface>/SKILL.md` exists + linked from README/llms.txt | **G1/G2** (new); today **N** (`AGENTS.md:53`) |
| Changed **public API** → doc + SKILL.md (same PR) | `pub` change / CLI flag / HTTP route / Py·JS binding | matching `skills/<surface>/SKILL.md` updated in the same diff | **G2** (new); today **N** (`AGENTS.md:42-53`) |
| New public **config/flag/env** → docs | new config key / `sparq-cli` flag / env var | documented in SKILL.md/README | **G6** (new) |
| New feature → **conformance/coverage** test | new headline feature in a crate | a conformance suite entry or a coverage/recall ratchet | **F** (cert epic) + **E** (coverage + test-presence ratchets already gate *that tests exist*) |
| Anything merged → coverage + test-presence | any change | per-crate line-% floor + test-count floor hold | **E** (`scripts/coverage*.py`) |
| New `unsafe` → Miri + justification | diff adds `unsafe` | ledger entry + crate in Miri lane | **G4** (new); today only informational `geiger` |
| New **ZK circuit member** → gate-count snapshot | new member under `zk/compose/` (or new `zk/` `bin` circuit) | baseline row in `gate_count_snapshot.json` (or an `exempt_circuits` entry) | **E/G5** (`snapshot_covers_every_member` enforces compose members; `snapshot_covers_top_level_circuits` extends coverage to every top-level `zk/` proving circuit — sq-ncvq.8, done) |
| ZK verifier / public-input change → re-anchor + soundness | change to `verifier.rs::reconstruct_public_inputs` / circuits | `forge_gates`+`differential_fuzz`+gate-count+soundness audit+zk-toolchain lane | **E/N** (`AGENTS.md:104-120`) |
| New **bench suite** → registry + dashboard row | new `bench/<suite>/` | `benchmarks.toml` id + `FEATURED_SUITES` row (or `featured=false`) | **G3** (new) + **F** (dashboard-row bead) |
| New competitor-relevant engine feature → **gather** follow-up | engine/store path change + `competitor-relevant` label | a `gather` bead (re-run `gather-competitors.sh`, refresh `competitors.json`) | **F** (un-gateable; out-of-band) |
| Parser change → conformance + fuzz | parser path | SPARQL/turtle conformance ratchets + fuzz lane | **E/N** (`AGENTS.md:104`) |
| Storage/encoding change → perf-gate + byte-diff + dict-spill cov + graph_open fuzz | store/dict/compress path | deterministic perf ratchet + differentials + fuzz | **E/N** (`AGENTS.md:104-120`) |
| SHACL change → W3C SHACL ratchet + diff-fuzz | `sparq-shacl` | core≥98 / sparql≥5 ratchet + pySHACL diff-fuzz | **E** |
| Cargo deps change → audit/deny/SBOM | `Cargo.toml`/`Cargo.lock` | supply-chain gate | **E** |
| `wasm` change → deps-guard + node test + bundle-size | `sparq-wasm` | `wasm-deps-guard.sh` + `wasm_bundle_bytes` | **E** |
| Doc describes code as it ISN'T → bead + fix | a doc claims "not implemented"/"TODO" about real code | convert to a bead, fix the doc | **N** (`AGENTS.md:122-127`); candidate **G** (grep for `TODO`/`not implemented` in `*.md` outside allow-list) |
| Research doc graduates → architecture/README/SKILL once implemented | a `research/*.md` design is now shipped | rewrite/fold into arch doc or README/SKILL | **N** (`AGENTS.md:122-127`); **F** candidate |
| New AGENTS.md convention → cross-pollinate sibling repos | edit to a portable "how we work" rule | file it in sibling repos' charter | **N** (`AGENTS.md`) |

The taxonomy confirms the steer: most *deterministic* rules are either already
**E** or become a thin new **G**; only the *out-of-band / judgment* follow-ups
(gather, dashboard row, cert plan, research-doc graduation) need **F**.

---

## 4. Existing machinery inventory (what the gates reuse)

So the design doesn't reinvent anything. (Full detail verified from the repo.)

- **`ci-summary / gate`** (`.github/workflows/ci-summary.yml`, sq-prg4): the
  single required check; dynamically polls all check-runs on the head commit;
  excludes `\b(advisory|informational)\b` (sq-wjth). New gates need only be a
  failing check-run.
- **Custom checks** (`scripts/`): `check-no-perf-numbers.py` (no hard-coded perf
  in tracked md; advisory), `check-readme-template.py` (per-crate README
  template, ≤120 lines + 3 emoji sections + License; advisory),
  `check-skill-frontmatter.py` (**hard**: every `skills/**/SKILL.md` has
  `name`+`description` frontmatter — wired in `docs-quality.yml`).
- **Ratchets** (committed floors, only rise): SPARQL conformance `RATCHET=1229`
  + inference `1967` + SHACL `core 98 / sparql 5` (literal constants in
  `ci.yml`); coverage line-% (`bench/coverage-floor.json`, `coverage-gate.py
  --check-robust`); test-presence (`bench/coverage-presence.json`,
  `coverage-presence.py`); deterministic perf (`bench/perf-baseline.json`,
  `perf-gate.py`, hard exit-2 for `mode:auto`, advisory for `mode:noise`).
- **ZK gate-count snapshot** (`crates/sparq-zk-compose/tests/gate_count.rs` +
  `gate_count_snapshot.json`, bead **sq-c5f** — note: the epic refers to this as
  "sq-0x65"; the real id is **sq-c5f**): `snapshot_covers_every_member` fails if a
  compiled member under `zk/compose/` lacks a baseline; `gate_count_regression`
  fails on circuit bloat past 3% tolerance.
- **AGENTS.md norms**: public-API→SKILL.md same-PR rule (`AGENTS.md:42-53`); the
  per-change re-evaluation table (`AGENTS.md:104-120`, with its own "keep in
  sync" note at 120); docs-stay-current (`122-127`); TODOs→beads (`60-70`);
  proactive AGENTS.md/skill maintenance (`144`).

---

## 5. Part 3 — drift findings (flow-on tasks already missed)

Grounded sweep (verified by listing/reading files). The full exhaustive sweep
becomes the reusable `scripts/drift-scan.py` (bead below); this is the concrete
initial list.

**A. Crates without benchmark coverage**
- `crates/sparq-nlq` — **zero benchmark** anywhere (no example bench, no
  `benchmarks.toml` entry) for the NL→SPARQL generate/repair loop.
- `crates/sparq-py` — **zero benchmark** for the PyO3 surface.
- `crates/sparq-serve` — the crate is **unbenched**; `bench/serve` is a detached
  spike (CATALOG labels it "research SPIKE, not a maintained regression
  benchmark"), not wired to the crate.
- `crates/sparq-mpc` — bench *examples* exist (`examples/mpc_bench_matrix.rs`,
  `mpc_net_bench.rs`, `mpc_party.rs`) but are **not registered in
  `bench/benchmarks.toml`** → invisible to CATALOG/CI/dashboard.
- `crates/sparq-wasm` — only the deterministic `wasm_bundle_bytes` metric;
  `js/bench/vs-oxigraph.mjs` exists but is **unregistered**.
- (Covered, not gaps: `sparq-solid` via `solid-wac-bench`, `sparq-parse` via
  `bench/parse`, core/engine/cli via CLI harnesses.)

**B. Public surfaces with no SKILL.md (or named in none)**
- `crates/sparq-sim` — entity similarity; **named in no SKILL.md** at all
  (has an example + `sim-olympics-eval` bench, zero public-skill surface).
- `crates/sparq-solid` — Solid WAC/ACP auth-view querying; **no dedicated
  `skills/solid/SKILL.md`** (only a passing mention inside `http-server`).
- `crates/sparq-gpu` — GPU kernels; **no skill** (experimental — may be
  intentionally internal; decide explicitly).
- `crates/sparq-zk-compose` — **not named in any SKILL.md** (`zk-query-proofs`
  references `sparq-zk` but not `-compose`); likely-stale gap.
- (Covered: reason→`inference`, rsp→`streaming-rsp`, serve→`http-server`,
  introspect→`genai-retrieval`, hdt folded into `data-formats`.)

**C. Docs drifted from code**
- **EXPLAIN asymmetry**: `explain`/`explain_analyze` exist in
  `crates/sparq-engine/src/explain.rs` and over HTTP
  (`crates/sparq-server/src/http.rs:1341`), but the **WASM/JS binding has no
  `explain` export** (`crates/sparq-wasm/src/lib.rs` — zero hits). The JS skill
  honestly omits it, so no *false* doc — but it's a public-surface capability
  asymmetry worth a bead (export EXPLAIN to JS, or document the omission).
- **`loadCompressed`: NO drift** — consistent across `sparq-wasm` ↔
  `js/src/store.ts` ↔ `skills/javascript-wasm/SKILL.md`.
- **Hard-coded perf numbers**: NONE in root `README.md`, `crates/*/README.md`,
  `docs/`, `llms.txt`, `AGENTS.md` — hygiene holding (numbers live only in the
  sanctioned `bench/*/README.md` + `research/`).

**D. Bench suites missing a dashboard row** (`FEATURED_SUITES` in
`bench/dashboard/dashboard.js:270` lists 11: LUBM, WatDiv, SP2Bench, Deep
Taxonomy, SHACL, Full-Text, GeoSPARQL, Vector/ANN, BSBM, DBPSB, Synthetic)
- **The entire ZK family is unfeatured** — `bench/zk` (`zk-commit-throughput`),
  `bench/zk-trace` (`zk-trace-overhead`), `bench/zk-compose`
  (`zk-compose-gates`, `zk-compose-prove-verify`): 4 registered ZK benches, **no
  dashboard row**, while every other capability surface is promoted. Highest-signal D gap.
- Also unfeatured: `selective-bindjoin`, `u64-valueids`, `operator-coverage`,
  `qlever-olympics`/`qlever-100m`, crate-example benches `hdt-load-bench`,
  `rsp-throughput`, `gpu-bench`, `sim-olympics-eval`, `introspect-olympics`.
  (Spikes serve/memtier and external-cost wikidata suites are acceptably unfeatured.)

**E. Features without central conformance coverage** (`crates/sparq-conformance`
covers SPARQL query/syntax/update, Turtle, inference RDF-MT/OWL2-RL/N3/entail)
- **SHACL** (`sparq-shacl`) and **GeoSPARQL** (`sparq-geo`) conformance live as
  **crate-local ratchet tests** (`crates/sparq-shacl/tests/{w3c_core,w3c_sparql}.rs`,
  `crates/sparq-geo/tests/ogc_compliance_ratchet.rs`) — gated, but **outside the
  central `sparq-conformance` scoreboard**, which therefore *under-reports* the
  project's real conformance surface. Either surface them in the central report
  or document the split intentionally.
- No conformance for RSP/streaming (RSP-QL community tests exist, none wired),
  ZK, MPC (no reference suite — note explicitly which are "no spec exists" vs
  "spec exists, unwired").

---

## 6. Bead graph

Children of epic **sq-ncvq** (created via `bd create`; ids + deps listed in the
report accompanying this doc). Structure:

- **Mechanism build** (parallel where independent):
  - flow-on-rules config + `flow-on.py` + `flow-on.yml` (the reactive layer);
  - one bead per proactive gate G1–G6;
  - the AGENTS.md table annotation ("enforced-by" column);
  - the reusable `drift-scan.py` + scheduled lane.
- **Rule codification**: fold the taxonomy (§3) into the AGENTS.md per-change
  table + each new gate's docstring.
- **Drift catch-up** (one per concrete gap, grouped by area): bench gaps (A),
  skill gaps (B), EXPLAIN-JS asymmetry (C), ZK dashboard row + others (D),
  conformance-scoreboard consolidation (E).
- **Optional upstream**: a low-priority beads "on-close lifecycle hook" proposal
  issue (not blocking). **Resolved (sq-ncvq.17):** not filed — upstream already
  shipped the on-close hook ([`gastownhall/beads#1754`][bd1754], CLOSED/COMPLETED);
  filing would duplicate a merged feature. See §1.4. [OPUS-4.8]

Dependencies: the drift-scanner (`drift-scan.py`) is a dependency-free leaf that
unblocks an *automated* full sweep; the gate beads are independent of each
other; the flow-on config bead blocks `flow-on.py` which blocks `flow-on.yml`;
the AGENTS.md-annotation bead depends on the gate beads existing (so it can name
them).

[bd1754]: https://github.com/gastownhall/beads/issues/1754
