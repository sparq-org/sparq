# CI runner consolidation — collapsing sub-minute jobs (2026-07)

> 🤖 **SPARQ agent** design record (CI-consolidation architect). Design only — this
> record changes no workflow; the implementation is tracked by a P1 bead under epic
> sq-6vshe (CI structural speedup). Companion records:
> `research/ci-structural-speedup.md`, `research/ci-gate-slotless-aggregation.md`,
> `research/ci-mergequeue-speedup-2026-07.md`.

## 1. Problem: runner claims dominate sub-minute jobs

Five workflows declare 33 jobs of which 31 finish in under a minute (sampled runs
29158759154 / 29158759111 / 29158759129: medians 10–20s, extremes 7–30s of real
work). Every job is a separate runner claim — queue wait + claim + checkout + action
setup — so the fixed overhead is paid up to 13 times per event for seconds of actual
linting. `docs-quality.yml` alone claims 13 runners on every `pull_request`,
`merge_group` **and** main push; it is the repo's highest-frequency workflow.

This churn is not just latency: it feeds the documented congestion-collapse mode
(see `reference-ci-congestion-collapse` + the sq-90cv4 adaptive saturation budget in
`scripts/ci_summary_gate.py`). The `gate` aggregator is a *waiter* holding a slot;
when tens of seconds-cheap jobs flood the pool across ~13 PRs, builds starve, the
waiter times out, and valid PRs get a false `gate=FAILURE`. Cutting ~24 runner
claims per CI event attacks the collapse at its source.

## 2. Why this is ruleset-safe (gate-contract analysis)

Branch protection requires exactly **one** check: `gate` (ci-summary).
`scripts/ci_summary_gate.py` **dynamically polls every check-run** on the head SHA
and fails iff any check-run whose name does **not** match
`\b(advisory|informational)\b` concluded failure. Check-runs are created **per
job** — steps do not create check-runs. Therefore jobs can be merged/renamed freely
without touching the ruleset, provided:

- **(a)** every currently-GATING check still runs, inside a job whose check-run
  name carries **no** advisory/informational whole word;
- **(b)** every currently-ADVISORY check still runs, inside a job whose name
  **keeps** an advisory token (and whose check-run concludes SUCCESS — findings
  swallowed at the step);
- **(c)** no job that a surviving sibling `needs:` is merged away.

Four **name contracts** exist beyond the token rule, all honored below:

1. `SELECT_RE = re.compile(r"change-based test selection")` in
   `ci_summary_gate.py`: a check-run matching that phrase must exist, be
   unconditional, and conclude SUCCESS — the merged fv job keeps the phrase.
2. `.github/advisory-registry.json` + `scripts/check-advisory-registry.py` (C2:
   every advisory-named **job** needs a registry entry; C3: no
   `scripts/*gate*.py` step inside an advisory-named job without a waiver). Any
   advisory-job rename ships the registry edit **in the same PR** — the checker
   runs inside docs-quality's own `ci-scripts` step, so the PR self-validates.
3. `scripts/tests/test_ci_summary_gate.py` pins that plural **"advisories"** (the
   cargo-deny job name) does **not** match the advisory token — consolidated
   supply-chain names keep that word-shape and stay free of the whole words
   `advisory`/`informational`.
4. `readme-template (template)` is a de-facto REQUIRED gate (its name deliberately
   carries no advisory token); it survives verbatim as a step name.

## 3. Shared consolidation semantics

- **Gating bucket**: one job per workflow; the former jobs become **sequential
  step-groups** carrying the exact former job name (log greppability + blame
  attribution). No `continue-on-error` anywhere in the bucket → any failing step
  fails the job → the single gating check-run reds → `gate` reds. Default
  fail-fast step semantics. (Optional refinement, not required by the contract:
  `if: ${{ !cancelled() }}` on independent lint steps so one red lint does not
  hide another; the job still concludes failure.)
- **Advisory bucket**: the job name carries the advisory token, and findings are
  swallowed **at the step** (`|| true`, step-level `continue-on-error: true` on
  vale/reviewdog, lychee `fail: false`) — job-level `continue-on-error` alone
  still concludes the check-run `failure` and would block the gate (the verified
  PR #42 failure mode documented in the docs-quality.yml header). Job-level
  `continue-on-error: true` is kept only as defence-in-depth.
- **Change detection** (`changes` jobs) folds to a **leading in-job step**; its
  step output gates the later steps via per-step `if:` (the zk-toolchain.yml
  pattern). Doc-only events: steps skip, the job concludes SUCCESS — still
  non-failing to the aggregator (previously the jobs were `if:`-skipped, also
  non-failing). `rust_changed` stays forced `true` off `pull_request`.
- **Kept standalone**: anything heavy, matrixed, differently-provisioned, or
  `needs:`-consumed by a survivor — the kani proof matrix and
  `artifact-exact-equality` (two full release-wasm builds, Swatinem cache).
- Setup dedup per bucket: one checkout (`persist-credentials: false`), one
  setup-node 22, one setup-python 3.12 + hash-pinned PyYAML
  (`.github/requirements/docs-quality.txt`), one SHA-pinned
  `taiki-e/install-action` per tool (typos, lychee; deny/vet/cyclonedx).
- Each consolidated job gets `timeout-minutes` sized to the sum of its former
  jobs' budgets; `concurrency` groups are unchanged (job count does not affect
  cancel semantics).

## 4. Per-workflow plans

### 4.1 docs-quality.yml — 13 jobs → 2

**Job `quick-gates`** — name `docs-quality quick-gates` (GATING, fail-fast). One
checkout + setup-node + setup-python/PyYAML + install-action (typos@1.47.2,
lychee@0.23.0), then the ten former gating jobs as step-groups, fastest greps
first, the big `ci-scripts` bucket last:

1. `markdownlint (docs)`
2. `typos (docs)` (incl. its allow-list self-test)
3. `internal-links (lychee --offline)` (lychee-action, offline)
4. `skill-frontmatter (Agent-Skills YAML)` (incl. self-test)
5. `privacy-claims (ZK/MPC honesty gate)` (incl. self-test)
6. `terminology (RDF 1.2 / SPARQL 1.2 wording)` (incl. self-test)
7. `no-perf-numbers (docs)` (`--enforce`)
8. `no-dyn-dispatch (substrate)`
9. `readme-template (template)` (incl. `--self-test`; de-facto REQUIRED gate — name pinned as step)
10. `ci-scripts (CI-helper self-tests + workflow lints)` (the existing ~40-step
    bucket, absorbed whole — it is already the in-file precedent for this design)

**Job `quick-advisory`** — name `docs-quality quick-advisory (advisory)`
(ADVISORY, never fail-fast, job-level `continue-on-error: true` as
defence-in-depth). Steps keep their exact step-level swallowing:

1. `markdownlint-advisory (whole repo)` — `|| true` on the lint, count summary
2. `vale (prose, advisory)` — **step**-level `continue-on-error: true` on the
   vale/reviewdog action
3. `external-links (lychee online, advisory)` — lychee-action `fail: false`

**Registry edit (same PR, mandatory)**: remove the three docs-quality entries
(`markdownlint-advisory (whole repo)`, `vale (prose, advisory)`,
`external-links (lychee online, advisory)`) from
`.github/advisory-registry.json` and add one entry for
`docs-quality quick-advisory (advisory)` (workflow `docs-quality.yml`, owner bead
`sq-5fd1`, per-step promotion criteria: a step promotes by **moving** into
`quick-gates`; external-links never promotes). C3 is clean — none of the three
steps invokes a `scripts/*gate*.py` script. C2 in `check-advisory-registry.py` is
one-directional (advisory-named job → entry), but stale entries are removed anyway
for hygiene.

### 4.2 flow-on-gates.yml — 4 jobs → 2

**Job `quick-gates`** — name `flow-on-gates quick-gates (G1 + G2 + G6)` (GATING,
fail-fast). One checkout (`fetch-depth: 0` for the merge-base diff) + one
setup-python, then:

1. Read PR labels **once** (shared step, `id: labels`) — the G2/G6
   merge_group-safe resolution (`scripts/resolve-merge-group-pr.py` primary, API
   fallback, loud warn + unsuppressed fail-closed) currently duplicated in both
   jobs, done a single time.
2. `new-crate-completeness (G1)` — `scripts/gate-new-crate.py --base "$GATE_BASE_REF"`
3. `public-api-skill (G2)` — consuming the shared labels output
4. `config-documented (G6)` — consuming the shared labels output, **keeping its
   three hermetic self-test suites** (`test_gates.py`,
   `test_check_new_bench_registered.py`, `test_flow_on.py`) as leading steps

**Job `new-bench-registry-dashboard`** — kept **verbatim** as its own tiny job,
name `new-bench-registry-dashboard (G3, advisory)`, script run with
`--advisory` (always exits 0). Rationale: its name is a pinned entry in
`.github/advisory-registry.json`; keeping it untouched means **zero registry
churn** and invariant (b) holds trivially. (Folding G3 as a step into the gating
job was rejected: it would mix advisory work into a gating-named check-run and
force a registry removal for no additional saving beyond one ~11s claim.)

### 4.3 formal-verification.yml — 3 jobs → 2

**Job `fv-select`** — **job id kept** (so kani's `needs: fv-select` and its
`fromJSON(needs.fv-select.outputs.suites)` matrix are untouched), name
`fv-select (change-based test selection) + fv-manifest (proof inventory)` — the
name **keeps the exact `SELECT_RE` phrase**, stays **unconditional** (no
`if:`/`needs:`), and still declares `outputs: any/suites` mapped from the selector
step. Steps, selector first so outputs are emitted before anything else can fail:

1. `fv-select (change-based test selection)` — selector `--self-test` + the
   `id: sel` selection step emitting `any`/`suites`
2. `fv-manifest (proof inventory completeness + drift)` — PyYAML (hash-pinned)
   install + `check-fv-manifest.py --self-test` + the manifest gate

Diagnosability trade (accepted): a manifest violation now reds the select-named
check-run instead of its own — still fail-closed, still blocks the merge, and the
failing **step name** preserves attribution. No golden/name test pins the
fv-manifest check-run name. If the merged job fails, kani is skipped — also
fail-closed, since the red check-run already reds `gate`.

**Job `kani`** — `kani ${{ matrix.suite.name }} (change-coupled proofs)`:
**untouched** (nightly toolchain, fail-fast:false matrix, 120-minute budget,
`if:`-skipped unless a `proves` path changed).

### 4.4 supply-chain.yml — 9 jobs → 1

**Job `supply-chain-gates`** — name
`supply-chain gates (deny + vet + SBOM + VEX + OpenSSF + js-sbom)` (GATING,
fail-fast; contains no whole-word advisory/informational token — "advisories"
plural in a step name is safe and pinned so by the gate's test fixture). All
`needs: changes` consumers merge into this same job, so invariant (c) is
satisfied with the `changes` job folded to a leading step. Steps — always-on
cheap checks first so doc-only PRs finish fast, Rust chain last:

1. `detect rust changes (supply-chain)` — leading in-job dorny/paths-filter +
   decide step (`rust_changed` forced `true` off `pull_request`), `id: changes`
2. `VEX ↔ deny.toml sync (GS-5) — GATING` (runner python, unconditional)
3. `OpenSSF Best-Practices self-cert evidence (GX-4) — GATING` (unconditional)
4. `JS/npm CycloneDX SBOM (published WASM client)` — setup-node 22 +
   `gen-js-sbom.sh` + upload artifact `sbom-js-cyclonedx` (unconditional)
5. Rust setup — dtolnay stable + `taiki-e/install-action` (cargo-deny,
   cargo-vet@0.10.2, cargo-cyclonedx) in one shot
6. `cargo-deny (advisories + bans + sources + licenses)` —
   `if: steps.changes.outputs.rust_changed == 'true'`
7. `cargo-vet (per-dependency audit attestations) — GATING` — same step-`if`
8. `generate CycloneDX SBOM` — **generated ONCE, unconditionally** (cargo fetch
   retry + cargo cyclonedx + jq path-normalize); upload artifact
   `sbom-cyclonedx` behind the `rust_changed` step-`if` (preserving today's
   artifact cadence exactly)
9. `SBOM purl-canonicality assertion (GS-6/GS-7) — GATING` — over the
   once-generated SBOM (unconditional, as today)
10. `SBOM per-component supplier-name assertion (GS-1) — GATING` — over the
    once-generated SBOM (unconditional, as today)

Semantics preserved: today `sbom-purl-canonical` and `sbom-supplier` **each**
regenerate the full workspace SBOM and run even on doc-only PRs; the consolidated
job generates once and asserts twice — strictly less work, identical coverage.
Both upload-artifact outputs survive with their names.

### 4.5 vectorized-feature-off.yml — 4 jobs → 2

**Job `quick-gates`** — name
`vectorized-feature-off quick-gates (changes + feature-resolution + cfg-audit)`
(GATING, fail-fast). No `needs:`. Steps:

1. `detect rust changes (vectorized-feature-off)` — leading in-job paths-filter
   + decide step, `id: changes`
2. `feature-resolution (vectorized OFF guard)` — step-`if` on `rust_changed`
   (stable Rust is preinstalled on ubuntu-latest; `cargo metadata` only)
3. `cfg-audit (vectorized call-site gates)` — same step-`if`

**Job `artifact-exact-equality`** — name kept verbatim
`artifact-exact-equality (wasm bundle feature-OFF)`; **drops `needs: changes`**
and gains its **own** leading in-job path-filter step (the same seconds-cheap
decision), with all existing heavy steps (fetch-depth 0, Swatinem rust-cache key
`vectorized-feature-off-wasm`, base-SHA keyed cache, the two release-wasm builds
+ byte-compare) behind the step-`if`. This keeps the long pole out of the merge
**and** starts it a few seconds *earlier* on Rust PRs (no `needs:` wait). Cost:
on doc-only PRs this job now claims a runner for its filter step (~7s) where it
was previously skipped for free — see §5.

## 5. Runner savings

| workflow | jobs before | jobs after | claims saved / full event |
| --- | --- | --- | --- |
| docs-quality.yml | 13 | 2 | 11 |
| flow-on-gates.yml | 4 | 2 | 2 |
| formal-verification.yml | 3 (2 execute steady-state) | 2 (1 executes) | 1 |
| supply-chain.yml | 9 | 1 | 8 |
| vectorized-feature-off.yml | 4 | 2 | 2 |
| **total** | **33** | **9** | **24** |

Notes:

- "Full event" = a Rust-touching `pull_request` run; docs-quality + supply-chain
  + vectorized also run on `merge_group` and main `push`, so a PR's lifecycle
  saves roughly 2–3× the per-event figure.
- formal-verification: kani is `if:`-skipped on most PRs (0 claims before and
  after); the steady-state saving is the fv-manifest claim.
- vectorized doc-only PRs regress by **+1 short claim** (artifact job's in-job
  filter) against **−2 claims** on every Rust-touching event; repo traffic is
  Rust-dominated, so net strongly positive. If this ever inverts, the fallback is
  `needs: quick-gates` + job-`if` (parity on doc-only, −1 on Rust, and the
  documented long-pole delay).
- Wall-clock: each gating bucket is the sum of seconds-cheap steps minus ~10
  duplicated checkouts/setups — a bucket lands well inside the CodeQL (~20–40min)
  and coverage (~17–19min) critical path, so `gate` latency is unaffected.

## 6. Hazard ledger

| hazard | mitigation |
| --- | --- |
| Advisory bucket's check-run concludes failure and blocks `gate` (PR #42 mode) | Findings swallowed at the STEP (`\|\| true` / step `continue-on-error` / lychee `fail: false`), carried verbatim from today's jobs; job-level `continue-on-error` as defence-in-depth only |
| advisory-registry C2 reds on the renamed advisory job | Registry edit ships in the same PR (remove 3 docs-quality entries, add `docs-quality quick-advisory (advisory)`); G3's job/name/entry untouched; the checker runs in the same PR's `ci-scripts` step, so the PR self-validates |
| advisory-registry C3 (gate script in an advisory job) | None of the 3 advisory steps invokes `scripts/*gate*.py` — verified |
| `SELECT_RE` contract broken by the fv merge | Merged job name keeps the literal phrase `change-based test selection`, stays unconditional, keeps `outputs: any/suites`; selector step runs first |
| kani `needs:`/matrix breakage | Job **id** `fv-select` unchanged; kani job untouched |
| A gating name accidentally gains an advisory token and silently stops gating | New gating names reviewed against `\b(advisory\|informational)\b`; plural "advisories" is pinned GATING by `test_ci_summary_gate.py` |
| Failure attribution coarsens (one check-run per bucket) | Former job names survive as step names; the failing step is the first red line in the single log |
| One red lint hides sibling lint results (fail-fast) | Accepted for v1 (matches contract); optional `if: ${{ !cancelled() }}` per independent lint step is a follow-up refinement |
| SBOM single-generation becomes a single point of failure | Generation failure fails the job → `gate` reds — fail-closed, same net effect as today's three independent failures |
| Artifact outputs disappear | `sbom-cyclonedx` (rust_changed cadence, as today) + `sbom-js-cyclonedx` (always) both preserved by name |
| Doc-only skip semantics change | Skipped-job → skipped-step; job concludes SUCCESS — non-failing to the aggregator either way |
| In-flight PRs racing these workflow files | Land as ONE implementation PR (five workflow files + the registry edit are one atomic rename set) |

## 7. Rollout

1. One implementation PR: the five workflow files + `.github/advisory-registry.json`.
2. Pre-push local checks: `python3 scripts/check-advisory-registry.py`,
   `python3 scripts/tests/test_ci_summary_gate.py`,
   `python3 scripts/tests/test_gates.py`, `actionlint`/workflow lints via the
   ci-scripts suite.
3. Verify on the PR's own run: the new check-run inventory shows exactly the §4
   names, the advisory bucket concludes SUCCESS with findings visible in its log,
   and `gate` goes green. Then watch one merge_group pass and one main push.
4. Follow-up candidates (out of scope here): the remaining multi-short-job
   workflows outside this batch, and the `if: ${{ !cancelled() }}` lint-step
   refinement. Candidates swept off this surface — folded or declined — are
   recorded in §8.

## 8. Fold-candidate ledger (the §7.4 sweep)

The §7.4 follow-up surface is swept candidate by candidate. A candidate is any
job whose runtime is dominated by the per-job constant tax (§1). Being that
shape is necessary but **not sufficient**: the sweep has produced both folds and
declines, and the declines are the more informative half, so both are recorded
here.

### 8.1 The two discriminators the sweep applies

§2's gate-contract invariants (a)–(c) say when a fold is *ruleset-safe*. They do
not say when it is *worth it*. Two further questions decide that, and they are
what separate the entries below:

1. **Does the fold merge two GATING identities?** Folding two advisory jobs
   cannot coarsen a gating verdict — the aggregator already excludes both.
   Folding two *gating* jobs replaces two independently-named red signals with
   one, and the surviving name then describes only one of the defect classes it
   can report. §6 books that as "failure attribution coarsens", mitigated by
   step names in the log; that mitigation is adequate *inside* a bucket of
   like-for-like lint steps, and inadequate when the merged checks assert
   unrelated properties that a human reads off the check-run name.
2. **Are the candidates in the same hermeticity + runtime class?** A grep over
   checked-in source and a real-browser run are different failure *populations*,
   not just different steps. When their budgets differ by an order of magnitude
   the fold additionally forces the ordering dilemma in 8.2 below.

The repo already encodes discriminator 2 as a pattern: `gui.yml` ·
`gui-hermetic-guards` bundles the `browserName` drift tripwire with the
`no-sleep-gate` grep — two hermetic greps in one claim — and was deliberately
hoisted *out* of the environment-coupled `tauri-e2e` job by #3773. Hermetic
greps cluster with hermetic greps.

### 8.2 Declined: `site-e2e-foundation.yml` · `determinism-gate` (issue #5692)

`determinism-gate` is checkout + `bash site/e2e/support/no-timeout-gate.sh` and
nothing else — the canonical §1 shape. It is declined anyway.

Its only same-trigger, unconditional, gating sibling is `a11y-ratchet` (the
third job, `foundation-smoke`, is declared advisory). Both fail discriminator 1
and discriminator 2:

- **Two gating identities.** Neither job is declared in
  `.github/advisory-registry.json`, so both gate, and
  `.github/E2E-GATING-POLICY.md` §2 lists them as two separate rows with two
  separate "why it gates" justifications (a hermetic grep vs a ratchet over
  axe's own rule output). After a fold, a `page.waitForTimeout` under
  `site/e2e/` reds a check-run named for the axe WCAG scan. That is not a log
  attribution problem — it is the gate-level name, and it is the name the policy
  document pins.
- **The ordering dilemma.** Put the grep first and fail fast, and a determinism
  failure suppresses the a11y result for that run — the exact narrowing #5221's
  `if: ${{ !cancelled() }}` guard was added to prevent. Add that guard, and the
  sub-minute determinism verdict is withheld until the browser lane concludes,
  because a check-run has one conclusion and it arrives when the job ends. The
  lane's own budgets state the gap: `determinism-gate` carries
  `timeout-minutes: 5`, `a11y-ratchet` carries `timeout-minutes: 20` on top of
  `npm ci` + a Playwright Chromium download. Today neither regression exists.
- **The saving is smaller than the §5 rows.** Those workflows also run on
  `merge_group` and main `push`, so §5 counts a PR lifecycle at roughly 2–3× the
  per-event figure. `site-e2e-foundation.yml` triggers only on `pull_request`
  and `push` to main, both path-filtered to `site/**` / `package.json` /
  `package-lock.json` / the workflow file. The saving is one claim on the subset
  of PRs that touch the site.

There is no in-class partner to fold it into instead: `determinism-gate` is the
only hermetic grep in the workflow, and folding across workflows would mean
carrying this lane's `site/**` path filter into a job triggered on a different
set — paying a claim on events that do not touch the site, which is the cost
this sweep exists to remove.

One short claim on a path-filtered event subset does not buy a permanently
misnamed gating signal plus a choice between two fresh regressions. #3773 split
this job out of the advisory job it was hiding inside precisely so its verdict
would be independently visible; folding it into a browser job would surrender
that independence for the smallest saving the sweep has priced. The decline is
recorded in the job's own header comment so a later sweep does not re-derive it.
