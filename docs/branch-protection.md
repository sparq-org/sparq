<!-- [OPUS-4.8] Governance: branch-protection doc-of-record (bead sq-41ey). -->
# Branch protection — `main`

This is the **doc-of-record** for the branch-protection ruleset on `main`. The settings
themselves are configured **out-of-repo** by the repository owner under
**Settings → Branches → Branch protection rules** (or a repository ruleset) on GitHub;
they cannot be expressed in a tracked file. This document records what those settings
should be, so the intended protection is reviewable and reproducible if the rule is ever
recreated.

## Protected branch

- **`main`** — the only long-lived branch. All changes land via pull request; direct
  pushes are disallowed for non-administrators. Repository administrators can always
  bypass the ruleset (see "Other settings"); the automated landing flow does not.

## Required status checks

There is exactly **ONE** required status check — the aggregator. The live ruleset's
`required_status_checks` rule lists a single context (`gate`, from the `ci-summary`
workflow) and sets `strict_required_status_checks_policy: false` — i.e. PRs are **not**
forced to be re-based up-to-date with `main` before merging. This is consistent with the
solo-maintainer reality (a single serialized merge train: branches are gated and merged
one at a time per `AGENTS.md`, so a strict up-to-date requirement would only add churn
without a second concurrent author to race against). The required check is:

| Required check (job name) | Workflow | What it gates |
|---|---|---|
| **`ci-summary / gate`** | [`.github/workflows/ci-summary.yml`](../.github/workflows/ci-summary.yml) | **The single gate.** Polls Actions workflow-runs plus external check-runs on the PR head commit and passes iff the newest run/attempt of every gating workflow succeeds (`success`/`skipped`/`neutral` are non-failing). |

> **Select only `ci-summary / gate`** in the ruleset's "Require status checks that must
> pass" list. Do **not** add the individual job names below — `ci-summary` already
> aggregates them. This is deliberate: `needs:` cannot span workflows, so requiring each
> job by name was brittle (every rename / added gate broke the rule and silently weakened
> the gate). The aggregator adapts automatically — add or rename jobs freely and the gate
> still covers them, because it discovers the live workflow/check set at run time. See the
> header of `ci-summary.yml` for the full semantics (newest-run resolution, bounded
> cancelled-run re-dispatch, stability window, and self-exclusion).

### What `ci-summary` aggregates (informational — do NOT add these individually)

The gate covers **every newest Actions workflow run** and every external check-run on the
head commit. As of this writing those expose the jobs below; this table is a map for
reviewers, **not** a list of required checks.

> **Advisory checks are non-gating only when DECLARED (#3773).** `ci-summary` excludes a
> check from the gating set **iff its name is a key in
> [`.github/advisory-registry.json`](../.github/advisory-registry.json)** — its conclusion,
> even `failure`, then never blocks a merge. **Everything else GATES**, whatever it is
> called: a new job needs no naming ceremony to gate, and a visibility-only job must be
> declared (with an `owner_bead`, `promotion_criteria`, `registered`, `workflow` and
> `job_id`) or it will gate. `scripts/check-advisory-registry.py` enforces the registry's
> integrity: C2 (a job whose *name* carries an `advisory`/`informational` token must be
> declared or lose the token), C3 (a gate-classified python/shell/node/npm command inside a
> declared job needs an explicit `gate_script_waiver`), C4 (each declaration binds to
> `workflow` + `job_id` and must equal that job's **current** name — so a rename cannot
> silently flip gating status; the renamed job gates, fail-closed).
>
> Two entry-shape rules the aggregator itself enforces, so an under-specified entry can
> never buy an exclusion the integrity checker would have rejected: **all five fields are
> required by `ci-summary` too** (an entry missing `workflow`/`job_id` declares nothing and
> the check keeps gating), and a key must carry **literal text outside any `${{ … }}`
> expression** — an expression-only key such as `${{ matrix.label }}` would match every
> check-run name, `gate` included, so it is refused. Frame the expression instead:
> `GUI build + clippy (${{ matrix.label }}, advisory)`.
>
> Until 2026-07-25 the aggregator instead inferred advisory status from the display name
> matching `\b(advisory|informational)\b` (sq-wjth). That was removed in #3773: it silently
> neutralised four real gates, because any job whose name happened to contain those words
> was dropped from the gating set wholesale. The name token is now diagnostic only, and the
> gate summary prints a loud note for any token-carrying check that has no declaration.

From the **CI** workflow (`.github/workflows/ci.yml`):

| Job name | What it gates |
|---|---|
| `build + test (workspace)` | `cargo build --workspace --all-targets` + `cargo test --workspace`. |
| `clippy (gate) + fmt (non-blocking)` | `cargo clippy --workspace --all-targets -- -D warnings` (the clippy gate; fmt is non-blocking until the one-time reformat lands). |
| `MSRV check (Rust 1.88, declared floor)` | `cargo check` on the pinned MSRV toolchain. |
| `W3C SPARQL conformance (ratchet >= 1229 pass+divergence)` | The W3C SPARQL conformance ratchet (never lower). |
| `W3C SHACL conformance (ratchet — core >= 98, sparql >= 5)` | The W3C SHACL core + SHACL-SPARQL ratchets. |
| `Inference conformance (ratchet >= 1967 pass+divergence)` | The RDFS/OWL-RL/N3/entailment + rdf-turtle inference ratchet. |
| `coverage ratchet + test-presence gate (per-crate)` | The per-crate line-coverage floor + the test-presence gate. **On `merge_group` this verdict covers the fast no-compile FLOOR gates only** — the instrumented per-crate line-% MEASUREMENT is demoted off the queue's blocking path (see *Coverage MEASUREMENT off the merge queue* below). Where the measurement *does* run, it is **changed-cone scoped** (sq-3dr4t): a PR re-measures only the changed crates and their transitive reverse-dep closure; a crate outside that cone is unchanged (as is everything it depends on) so its floor verdict is inherited from `main` — reported as `INHERITED`, not silently dropped. Any full-run trigger (`Cargo.lock`, root `Cargo.toml`, `.github/`, `scripts/`, an unowned path — which includes `bench/coverage-floor.json`) or any selector error measures everything, and the nightly full run on `main` is the drift backstop. |
| `wasm build (sparq-wasm)` | The `wasm32-unknown-unknown` build, the wasm-deps guard, and `wasm-pack test --node`. |

From the security / supply-chain / SAST workflows (aggregated by the gate; all LIVE except
CodeQL, which is operationally disabled via manual workflow-disable — see the OPERATIONALLY DISABLED note):

| Job name | Workflow | What it gates |
|---|---|---|
| `cargo-deny (advisories + bans + sources + licenses)` | `.github/workflows/supply-chain.yml` | `cargo deny check bans sources licenses` (gating); advisories informational until cargo-deny ships CVSS-4.0 support (the daily `dependency-monitoring.yml` is the real advisory watchdog). |
| `generate CycloneDX SBOM` | `.github/workflows/supply-chain.yml` | The CycloneDX SBOM artifact. |
| `CodeQL analysis (rust)` | `.github/workflows/codeql.yml` | CodeQL SAST (`security-and-quality`) over the Rust workspace — resolves Scorecard's `SAST` check. |

> **CodeQL is currently OPERATIONALLY DISABLED (2026-07-18).** The rows and later
> sections below that describe `CodeQL analysis (rust)` as a live, gating per-PR
> check-run reflect the *ruleset's* intent, but the `codeql.yml` workflow is disabled
> via `gh workflow disable` (Actions workflow state `disabled_manually`): the file is
> retained on `main` with live triggers, but GitHub schedules no run, so no CodeQL
> check-run is produced on any event and it neither runs nor gates today. Open PR
> #3427 owns the successor policy (an advisory / retroactive posture) and the file's
> triggers are left untouched here to avoid colliding with it. Read every "CodeQL …
> gates" statement in this document against this note. (See *Merge-queue subset* below.)

From the binding/packaging workflows (when those surfaces are exercised):

| Job name | Workflow | What it gates |
|---|---|---|
| `maturin build + pytest` | `.github/workflows/python.yml` | The `sparq-rdf` PyPI binding (`import sparq`) build + pytest parity suite. |
| (js binding job) | `.github/workflows/js.yml` | The `@sparq-org/sparq` npm build/tests. |

> **Benchmarks — the DETERMINISTIC ratchet gates on PRs; the NOISY timing is nightly (sq-6vshe.6,
> maintainer-directed).** On a `pull_request` the `bench.yml` `run + track benchmarks` job runs the
> FAST DETERMINISTIC form only (`ci-bench.sh --deterministic-only`): the byte-count / memory-layout
> ratchet (store/dict/wasm bytes — a pure function of the code, immune to shared-runner noise) IS
> aggregated by the gate (it carries no advisory-registry declaration) and hard-fails the gate on a real
> regression. `bench.yml` no longer triggers on `merge_group` at all — the deterministic ratchet
> already ran on the PR head, re-runs on push-to-main, and the merged-tree wasm-feature-OFF invariant
> is independently guarded on `merge_group` by `vectorized-feature-off.yml`'s `artifact-exact-equality`
> leg — so the bench check simply does not appear on the merge_group ref and the gate never waits on
> it (the required set is the single `gate` context, not this job by name). The NOISY wall-clock timing
> suites (query latencies + the well-known sp2b/dbpsb/watdiv/bsbm/lubm + cargo-only latency suites)
> were dragging the merge queue and flapping the gate on shared runners, so they are RELOCATED to the
> EC2 full-suite lane (`bench-ec2.yml` `nightly-full-bench`, quiet dedicated spot instance) which
> publishes the full at-scale series to the `benchmark-data` branch the Pages dashboard reads.
>
> **[OPUS-5] #3784 — `bench-ec2.yml` is MANUAL DISPATCH ONLY, and the at-scale trend is on hold.**
> Both of that workflow's crons are RETIRED. It authenticates through an AWS OIDC role
> (`vars.AWS_BENCH_ROLE_ARN`) that the orchestration design deliberately descoped, so every scheduled
> tick failed in `Configure AWS credentials (OIDC)` *before any benchmark ran* — and because the check
> name carries no advisory token and no `.github/advisory-registry.json` declaration, that failure
> **GATED** (see §ADVISORY MUST BE DECLARED in `scripts/ci_summary_gate.py`), which is why `main` could
> not go green. The lane was RETIRED rather than declared advisory: muting a permanently-broken gate to
> green a dashboard is exactly what the declared-not-inferred rule exists to prevent. Consequences to
> read honestly: the at-scale timing series is **not** being collected on a cadence (the earlier
> "perf-tracking is moved, not lost" claim no longer holds), and maintainer-run EC2 benchmarking is
> unaffected — both campaigns stay dispatchable via the `lane` input, and every EC2 bench script stays
> in the tree. Reviving the cadence means re-provisioning the role AND re-adding a `schedule:` block in
> the same change. A cron-less workflow posts no check-runs on a `main` head SHA, so neither EC2
> campaign can gate anything today; release/dist workflows likewise remain non-gating (they fire only
> on tags). The **Scorecard** workflow
> (`scorecard.yml`) re-scores posture on push to `main` and feeds the public OpenSSF
> dashboard/badge (its SARIF upload to GitHub code-scanning is disabled and the job has
> no `security-events: write` — see the Scorecard note later in this document); with
> **CodeQL** operationally disabled (manual workflow-disable — see the OPERATIONALLY
> DISABLED note), the only current GitHub code-scanning feeders are the Trivy
> SARIF uploads (`container-scan.yml` on PR/main/schedule; `release.yml` for
> released images) and no `CodeQL analysis (rust)` per-PR check-run is produced.
>
> **No branch-protection ruleset change is required for this benchmark relocation.** The live ruleset
> requires exactly one context (`gate`), never the `bench` job by name, and the aggregator discovers the
> live check set at run time — so removing bench from `merge_group` and narrowing its PR run to the
> deterministic ratchet needs no ruleset edit. (If the maintainer had ever added the bench job by name
> to the required-checks list, THAT name would now need removing — but per the "select only
> `ci-summary / gate`" rule above, it was never added.)

### Merge-queue subset — the maintainer-directed risk posture (2026-07-18)

<!-- [FABLE-5] PR #3511 review finding 6: this records the DECISION so a future
     reviewer sees the reduced merge_group check-set is deliberate, not an accident. -->

The `merge_group` ref does **not** run an identical check-set to a PR. By explicit
maintainer direction (2026-07-18) the merge queue runs only the **PR-relevant subset**
of lanes, and — by a separate maintainer direction — **CodeQL is operationally
disabled**: the `.github/workflows/codeql.yml` FILE is retained on `main`
BYTE-IDENTICAL to before, with its full trigger set
(`pull_request`/`push`/`merge_group`/`schedule`) UNTOUCHED, but the workflow itself was
disabled via `gh workflow disable` (Actions workflow state `disabled_manually`), so
GitHub does not schedule it on any event — no CodeQL check-run is produced on ANY
trigger, so it neither runs nor gates. (This is an inert-file state, NOT an edit to the
triggers — this PR does not modify codeql.yml at all; open PR #3427 owns the codeql.yml
successor policy — an advisory / retroactive posture — so the file's triggers are left
untouched here to avoid colliding with it.)
Heavy or independent lanes that already ran and gated on the PR head dropped their
`merge_group` trigger because the queue re-run added wall-clock per enqueue with no new
signal: currently `formal-verification.yml` (Kani proofs), `fuzz.yml` (corpus replay),
`zk-toolchain.yml` (Noir forge suite), `container-scan.yml` (trivy image build + scan),
`supply-chain.yml` (cargo-deny/vet/SBOM), and `bench.yml` (noisy timing suites; the
deterministic ratchet is guarded separately — `bench.yml`'s removal predates this
directive, under sq-6vshe.6). This is a **decision, not a defect**.

<!-- [OPUS-5] issue #5165: the three zk/container/supply-chain lanes carry the same
     "(2026-07-18 maintainer directive, merge-queue subset) merge_group REMOVED" header
     comment as the other two but were missing from this list, which is the doc-of-record
     for the subset. Verified against the `on:` blocks in .github/workflows/ on
     2026-08-01; the same pass corrected research/ci-mergequeue-speedup-2026-07.md §2.1. -->

The lanes that DO trigger on `merge_group` today are: `ci-summary.yml` (the gate itself),
`ci.yml`, `feature-matrix.yml`, `vectorized-feature-off.yml`, `docs-quality.yml`,
`flow-on-gates.yml`, `routing-self-tests.yml` and `pr-area-label.yml` — plus
`codeql.yml`, whose trigger set lists `merge_group` but which is operationally disabled
per the note above and so produces no check-run there.

Why it stays sound:

- The gate polls whatever sibling check-runs actually exist on the ref and requires only
  the single `gate` context — a lane absent from `merge_group` is *never scheduled*, so
  it is never "expected but missing" and the gate never hangs on it.
- Every subset lane still runs and gates on the **PR head** (and the draft→ready-for-review
  re-run), so a real break is caught **before** admission for the code that changed.
- The **safety net for a lane the queue skips is POST-MERGE detection**: each such lane
  runs on `push`-to-`main`, backed by `nightly-full-sweep.yml` and the
  `formal-alarm.yml` / selection-alarm liveness monitors. A break that only a queued
  *combination* of PRs could produce is therefore detected on `main`, then recovered by
  **revert / fix-forward**.
- **There is no bisection.** `batch-merge.yml` explicitly states bisection is a v1
  unimplemented item; the recovery mechanism is revert/fix-forward plus the age-bound
  liveness backstop, not automated bisection of a failed batch. (Workflow comments that
  previously called batch-merge "the bisection recovery net" were corrected in PR #3511
  finding 6b to say post-merge detection + revert/fix-forward.)

The residual risk this accepts: a defect that manifests only in a *specific queued
combination* of PRs (not on any single PR head) reaches `main` and is caught post-merge
rather than pre-merge. The maintainer accepted this trade against per-enqueue wall-clock
and shared-runner contention.

### Coverage MEASUREMENT off the merge queue (sq-6vshe.17, 2026-07-29)

<!-- [SONNET-4.6] sq-6vshe.17 — LEVER 4a of research/ci-mergequeue-speedup-2026-07.md
     §3.4a. Recorded here (not only in the workflow comments) because it changes what a
     green merge-queue coverage check MEANS. -->

**Measured motive.** The instrumented per-crate coverage shards are the merge-group *entry
pole* whenever change-based selection skips the test shards: in the profiled runs cited in
`research/ci-mergequeue-speedup-2026-07.md` §2.2 a single coverage shard *was* the entry
critical path. (That record's timings are a dated snapshot of shared-runner CI, kept there
rather than restated here — steering data, not a canonical performance claim.)

**Why this one is demotable at all.** Coverage is a **ratchet, not a correctness test**. A
floor regression that slips through a queued batch is *detectable and recoverable*
post-merge; a functional bug is not. That asymmetry — and nothing about wall-clock — is
what makes the demotion designable.

**The enforcement topology** (`ci.yml`):

| Where | What runs |
|---|---|
| `pull_request` (non-draft) | **UNCHANGED — the primary gate.** A PR measures per-crate line-% and enforces the floor, over its **changed cone** (sq-3dr4t): the changed crates plus their transitive reverse-dep closure are re-measured, and a crate outside the cone — unchanged, as is everything it depends on — inherits its floor verdict from `main` (reported `INHERITED`). A full-run trigger or any selector error still measures everything. |
| `merge_group` | The `coverage-measure` / `coverage-engine-run` / `coverage-engine-merge` legs are **skipped**. `coverage-floors` — the fast, no-compile test-presence + floor-**monotonicity** + shard-partition gates — **still runs**, so a batch can never *lower* a committed floor. The `coverage` aggregate still concludes (a skipped leg counts as satisfied), so the gate never sees an expected-but-missing check; its step summary says plainly that only the floor gates ran. |
| `push` to `main` | **UNCHANGED, and now load-bearing** — this is the enforcement point for the batch-stacking case. It is deliberately **EXEMPT** from the sq-6vshe.14 push-run skip — an exemption now also written into that lever's own design (`research/ci-mergequeue-speedup-2026-07.md` §3.1 KEEP-list), so it is designed rather than discovered. It is pinned in `scripts/tests/test_ci_select_wiring.py::TestCoverageMergeGroupDemotion` against **both** shapes the skip can take: narrowing the legs' event envelope (behavioural, `test_measure_legs_still_run_on_a_pr_head_and_on_push_to_main`) and adding a `queue-validated` pre-job upstream of them via `needs:`/`if:` (structural, `test_measure_legs_take_no_new_upstream_gate` — the shape §3.1 actually specifies, and one the behavioural evaluation cannot see, since an absent context path is null there exactly as on GitHub). So landing that lever over the coverage legs REDs a test rather than silently removing the last enforcement point. |
| a `main` coverage red | `coverage-demoted-filer` auto-files a **P1 bead + a deduped GitHub issue** (`[demoted-lane] lane=coverage-ratchet-main`) via `scripts/ci-file-demoted-lane-failure.py` — the same demotion auto-bead protocol the fuzz and heavy-recall demotions use — **and** that open issue **pauses further ratchet ADVANCES**: `coverage-gate.py --check-advance-allowed` (a step in `coverage-floors`) fails a branch that *raises* a floor while the alarm is open, because the measured numbers a raise cites are exactly the numbers in doubt. It never blocks the recovery path (a governed lowering under `--allow-lower`), never blocks adding a *new* crate row, and fails **open** if the alarm probe is unavailable. |

**The residual risk, stated honestly.** Two PRs that each individually sit at or above
their floors can merge to a combined tree that sits below one. That case is **rare**, is
caught on the next `push` to `main` (order of ~15 minutes later), and is recovered by
revert / fix-forward — the same recovery mechanism the rest of this subset relies on.
What is **not** risked: no committed floor is ever silently lowered (the monotonicity gate
never left the queue, and a deliberate lowering stays a governed, loud re-baseline), and
the nightly full-coverage tier (`coverage-nightly`) is untouched.

## Draft-tier CI (reduced matrix on draft PR heads)

<!-- [FABLE-5] Draft-tier CI design record (2026-07-17). Motivation: the autonomous
     fleet keeps many draft worker PRs cycling review-fix rounds; every push ran the
     FULL matrix, saturating the org runner pool (gates timing out as false failures,
     the merge queue starving, the load-aware heavy shards deferring). -->

CI is **tiered by the PR's draft state**. A **draft** `pull_request` head runs a
REDUCED sibling set; a **non-draft** `pull_request` head, `push`-to-main, and every
scheduled/dispatch run keep the FULL matrix, byte-identical to before. (`merge_group`
is a SEPARATE axis: it always runs at FULL tier — never draft-tier — but it runs the
maintainer-directed **PR-relevant subset** documented in *Merge-queue subset* above —
several heavy lanes do not trigger there, and CodeQL (though its trigger set still
lists `merge_group`, byte-identical to main) is operationally disabled and so produces
no check-run there either — not a byte-identical copy of the PR check-set.)

**What a draft head runs:**

- the **change-scoped crate legs** — the existing `ci-select` change-based selection
  (affected reverse-dependency closure) already intersects every wide lane and the
  opt-in feature-matrix legs with the diff, on both tiers;
- the **cheap global gates** — clippy/fmt, MSRV, docs-quality (typos / privacy /
  ci-scripts), supply-chain, conformance ratchets for affected crates, pr-title;
- the **`ci-summary / gate` aggregator**, which evaluates exactly the reduced set it
  discovers (it is discovery-based, so no expected-leg list needs maintaining).

**What a draft head skips** (each re-runs at full tier before any merge is possible):

| Skipped on drafts | Where | Kept when |
|---|---|---|
| coverage ratchet (measure + engine split + aggregate) | `ci.yml` | never on drafts. The `ready_for_review` run re-measures at full tier; since sq-6vshe.17 the `merge_group` run does **not** re-measure (only the fast floor gates run there — see *Coverage MEASUREMENT off the merge queue*), so the non-draft PR head and the post-merge `main` run are the measurement points |
| benchmarks (deterministic ratchet + PR comparison/alert comments) | `bench.yml` | never on drafts |
| `cargo-fuzz` corpus replay (nightly toolchain + a libFuzzer build of every `fuzz/fuzz_targets/` target) | `fuzz.yml` `fuzz` | kept iff the PR carries `ci-full`/`fuzz-full` (`fuzz-full` also selects the randomized budget, so a bare draft skip would neuter it); otherwise the `ready_for_review` run re-replays at full tier. `differential-smoke` — the wrong-answer gate in the same workflow — is deliberately NOT draft-skipped: a wrong-answer regression is review-relevant |
| CodeQL analysis | `codeql.yml` | never on drafts (push-main + weekly schedule + merge_group + the ready_for_review run keep the `code_scanning` rule fed *when the workflow is enabled*; the merge_group run is since sq-g25hr additionally class-gated — an inert batch produces no analysis, a batch with any Rust does — while push-main and the weekly schedule always analyse in full. The workflow is currently operationally disabled (`disabled_manually`), so no CodeQL check-run is produced on any trigger today; open PR #3427 owns the successor policy) |
| heavy recall shards (`heavy-diskann`/`heavy-hnsw`) | `ci.yml` `test` | never on drafts (same demotion mechanism as their merge_group demotion) |
| wasm bundle build | `ci.yml` `wasm` | kept iff a wasm-bundle crate is in the affected closure (the existing lane-seed guard — unchanged on both tiers) |
| `artifact-exact-equality` (wasm feature-OFF byte identity) | `vectorized-feature-off.yml` | kept iff `sparq-wasm` is in the affected closure (in-step `ci_select.py` verdict; ci-full label / selector error / full mode ⇒ run) |

**The integrity invariant — a draft-tier gate result must NEVER admit a PR to the
merge queue.** The load-bearing mechanism is rule 1 (structural); rules 2–8 are
belts, and rule 9 is diagnosis + a recorded decision rather than a belt (all in
`scripts/ci_summary_gate.py`, unit-tested in
`scripts/tests/test_ci_summary_gate.py`; the name/trigger wiring pinned by
`scripts/tests/test_ci_select_wiring.py`):

1. **A draft-tier run never produces the required context at all.** The
   `ci-summary` gate job's own check-run name is **tiered**: a draft
   `pull_request` payload renders **`gate, draft-tier`**; every other event/state
   renders exactly `gate` — the sole context named by the ruleset's
   `required_status_checks` rule. A draft-built head therefore carries **no
   `gate` check-run at all**, and branch protection blocks on the *missing*
   required check from the un-draft moment until the full-tier run's fresh
   `gate` concludes. There is no supersession window to race: `gh pr ready &&
   gh pr merge --auto` (the fleet's standard flow) arms and waits; GitHub event
   latency, a *dropped* `ready_for_review` event, or an Actions outage all
   leave the required context ABSENT — blocked — never satisfied by a
   draft-tier result. Stale `gate, draft-tier` check-runs are tier artifacts:
   the gate script excludes them from every sibling set (a completed draft-tier
   verdict, green or red, is superseded by the live full-tier evaluation).
2. **Supersession by re-run.** Every gate-feeding workflow's `pull_request` trigger
   now includes **`ready_for_review`** (the default types are only
   opened/synchronize/reopened — without this the un-draft moment would run
   *nothing* and the head would keep its draft-tier results). Un-drafting therefore
   fires a FULL-tier run on the **same head SHA**, which produces the first (and
   only) `gate` check-run for that head.
3. **The gate knows its tier.** Each `ci-summary` run derives its tier from its own
   trigger payload (`PR_DRAFT`), and the reusable `ci-select` job's check-run **name
   carries a `", draft-tier"` marker** on draft-assembled runs (name-as-contract —
   the one such contract left, now that #3773 removed the advisory NAME rule).
4. **A full-tier gate refuses a draft-tier leg set — per INSTANCE.** `ci.yml`,
   `bench.yml`, `feature-matrix.yml` and `fuzz.yml` all call the same reusable
   `ci-select` job, so a head SHA carries up to **four** draft-marked selects
   under the IDENTICAL check-run name. A full-tier `pull_request` gate requires
   each draft-marked instance to have its **own, distinct, strictly-later**
   full-tier successor (greedy start-order matching) — the first workflow's
   full-tier select can never release the hold for the other three, whose
   full-tier runs may not have registered any check-runs yet. While any
   instance lacks a successor the set is *still settling* (the
   ready_for_review re-runs are expected); at budget exhaustion the verdict is
   **FAILURE — "stale draft-tier run, full run pending"**, never a pass over
   draft legs.
5. **A draft-tier gate re-checks the PR at conclusion time.** Before emitting
   SUCCESS it re-reads the PR's **current** draft state from the API
   (`pull-requests: read`); if the PR was un-drafted meanwhile it concludes
   FAILURE with the same stale-draft-tier message, and an unreadable state
   fail-closes to FAILURE (a draft PR cannot merge anyway).
6. **Newest workflow run wins (#3505).** [GPT-5.6] The ready_for_review re-run's
   per-PR concurrency groups cancel the in-flight draft-tier runs, leaving terminal
   checks on the same SHA. The gate resolves them by `workflow_id`: only the newest
   run (by creation/run id) and its newest `run_attempt` is authoritative. Every
   older run is a non-event even if its conclusion was `cancelled` or `failure`; a
   genuine failure in the newest run/attempt still REDs. A newest cancelled run is
   re-dispatched once (`actions: write`); if the retry is cancelled or never
   advances, the gate REDs loudly with `superseded-legs, re-run required`.
   Attempt-scoped job listing supplies the completed leg inventory and prevents
   an old attempt that reused the run id from leaking into the verdict; the
   workflow-run conclusion supplies the verdict when an entire job evaporates.
7. **A run that assembled NO LEGS is not evidence (#3781).** [OPUS-5] A
   `labeled` `pull_request` event whose label is not
   `ci-full`/`bench-full`/`fuzz-full` is a guarded no-op for every `ci-select`
   caller: the #2546 label-trigger guard skips every root job of `ci.yml`,
   `bench.yml`, `feature-matrix.yml` and `fuzz.yml`, so the run's ONLY non-skipped
   job is the deliberately-unconditional `select` pre-job. `ci-select.yml`
   therefore names that job **`…, no-leg`**, never `…, draft-tier`, and the gate
   treats the whole run as **non-authoritative**: it is excluded from newest-run
   candidacy (rule 6) and every check-run it produced is a non-event, so the
   PREVIOUS real run of that workflow stays authoritative. Two things this fixes,
   both measured on 2026-07-25 (#3472/#3468/#3681):
   * **the deadlock.** The review pipeline re-drafts a freshly-readied worker PR
     ~13 min after the ready and flips `review:needs` in the same breath. Before
     this rule, each of the resulting no-op runs' selects came out draft-marked
     (the PR was a draft again), so the head acquired four draft-marked instances
     whose full-tier successor could never exist — only a NON-draft payload
     produces one. Rule 4 then held for all 155 polls and refused a leg set with
     **zero failing legs**, three times in one drain pass.
   * **evidence erasure.** Newest-run resolution is per-workflow, so a vacuous
     all-`skipped` run used to *become* authoritative — discarding a real run that
     was still in flight (#3472's real CI matrix ran until 07:34:08, six minutes
     after the flip runs completed) or even a real `failure`. Ignoring the run is
     therefore strictly MORE fail-closed than the behaviour it replaces.
   Conservative by construction: on a `ci-full`/`bench-full`/`fuzz-full` flip at
   least one caller does real work, so no `no-leg` claim is made and the previous
   behaviour stands. `scripts/tests/test_ci_select_wiring.py::TestNoLegMarkerWiring`
   EVALUATES the marker expression against synthetic payloads and proves, by
   `needs:`-graph reachability, that every caller job really is inert on such a
   flip — the claim, not just the string.
   [OPUS-5] **#5215: only label ADDITIONS reach here now.** `unlabeled` was removed
   from the `on.pull_request.types` of `ci.yml`, `feature-matrix.yml`, `bench.yml`,
   `fuzz.yml` and `vectorized-feature-off.yml`, so a label REMOVAL starts no run at
   all — halving this no-op class, because a `review:*`/`status:*` state transition
   is a remove+add pair. Sound because every label those workflows read is a
   MONOTONE OPT-IN TO MORE WORK (`ci-full` ⇒ `ci_select.py --full`, `bench-full` ⇒
   the well-known suites, `fuzz-full` ⇒ the randomized budget): removing one can only
   ask for LESS than the run already standing on that head SHA, so skipping the
   removal event leaves a strict superset of the required coverage. The marker
   expression still handles `unlabeled` verbatim, so restoring the trigger needs no
   other edit. Practical consequence: **toggling `ci-full` OFF is no longer a way to
   force a re-run** — apply a label (or re-run the workflow) instead. Both halves —
   the trigger sets and the no-negative-label-read invariant that makes them sound —
   are pinned by `scripts/tests/test_label_trigger_economy.py`.
8. **An unsatisfiable hold REDs immediately, with the diagnosis (#3781).**
   [OPUS-5] Rule 4's hold is a WAIT for the `ready_for_review` full-tier re-runs.
   When every sibling has CONCLUDED, a draft-marked select still lacks a successor,
   and the PR reads as CURRENTLY A DRAFT, that wait is unsatisfiable on arrival, so
   the gate fails fast naming exactly that instead of burning the remaining budget
   (measured: 155 polls / ~37 min per occurrence). It is the same refusal rule 4
   reaches at budget exhaustion — never a new pass — and it stands down whenever the
   set is still settling, the PR reads non-draft, or the draft state is unreadable
   (then the pre-#3781 budget-exhaustion path renders the verdict unchanged).
9. **Both draft-tier refusals say that re-running the gate is futile, and the
   idle-head case deliberately has no exit (#4614).** [OPUS-5] Two carry-overs from
   the superseded #3765:
   * **Re-run futility is stated, not implied.** Rule 8's fast fail and rule 4's
     budget-exhaustion belt both end with the shared `UNSAT_HOLD_REMEDY` tail
     (`scripts/ci_summary_gate.py`): re-running `ci-summary` re-runs no *selecting*
     workflow, so a bare `gh run rerun` cannot make the missing full-tier select
     appear — only `gh pr ready` or a new head commit does. The audience is the
     automated repair lanes, whose reflex for any RED is otherwise "re-run it".
   * **The idle head gets no exit — decided, not overlooked.** Rule 8 fires only on
     a live draft read of `True`. If the PR reads NON-draft, or the draft read
     fails, the gate polls to the absolute budget. That is deliberate: the exit is
     licensed by a causal fact (only a non-draft payload produces a full-tier
     select), and no equivalent fact exists for an idle head — the successor may
     merely be late. #3765's alternative, a head-activity probe counting
     non-terminal Actions runs on the head SHA, is circumstantial evidence and
     would buy speed by introducing a new false-RED mode on PRs with zero failing
     legs. Burning the budget is slow; a false RED is a regression. Prior art if
     revisited: closed branch `fable/gate-unsatisfiable-hold-3758` at `dc92b4af`.

**Why the queue can never latch a draft-tier result.** Rule 1 is structural: the
queue and branch protection admit a PR only on a successful check-run of the
exact context `gate`, and no draft-tier run ever emits one. This matters more
than it may look, because the `merge_group` run deliberately omits two lanes
(`bench.yml`'s deterministic byte ratchet and the heavy recall shards,
sq-6vshe.6) on the premise that their full form already ran on the PR head —
a premise a draft-built head would otherwise break. Rules 2–8 alone would NOT
close that: a concluded draft-tier `gate` success would remain the *latest* run
of the required context from the un-draft moment until the ready_for_review
`ci-summary` run registers its check-run (seconds of event latency; indefinitely
if the event is dropped), and `gh pr ready && gh pr merge --auto` could enqueue
inside that window. With the tiered check name the window does not exist —
so the ready_for_review full-tier PR run (which includes both merge_group-absent
lanes) must conclude before the queue can admit the head.

The ruleset additionally carries a `code_scanning` rule (CodeQL). While CodeQL
is operationally disabled (manual workflow-disable — see the OPERATIONALLY DISABLED note) no
analyses are produced, so this rule currently exerts **no** blocking pressure.
If CodeQL is re-enabled: a draft-built head carries no PR CodeQL analysis
(`analyze` skips on drafts), and — since sq-g25hr — neither does a `merge_group`
batch whose change-class is proven inert (`scripts/ci_select.py --classify-only`
returns one of `_INERT_CLASSES`: the queued diff touches no Rust at all. Any
`crates/**` path in the batch classifies `mixed` and the full analysis runs, and
`push`-to-main analyses every merged commit in full, so the default branch's
code-scanning database never goes stale). The rule would then *independently*
block such a head
— but it is an **out-of-repo, owner-mutable setting** and evadable in corner
cases (a non-draft PR sharing the same head SHA supplies an analysis for the
commit; the rule may be relaxed during a CodeQL outage), so it is recorded here
as **defense-in-depth only**, never the load-bearing mechanism. Do not weaken
rule 1 on the strength of it.

**Operational notes.** `pull_request`-event CI runs are cancel-superseded per PR
(`concurrency` groups; `bench.yml` now cancels superseded **PR** runs only — its
push/schedule runs still never cancel, protecting the benchmark history — and
`js.yml` gained the standard per-PR group). `merge_group` runs are never cancelled
by these groups. **No required-check name changed** — the ruleset still requires
exactly `ci-summary / gate`, and every full-tier run still emits exactly that
context; a draft-tier run emits the additional, deliberately **non-required**
context `gate, draft-tier` instead (tooling reading a draft head's checks sees
the tier verdict there, while the required `gate` context stays absent until
un-draft — a draft PR cannot merge regardless). Toggling draft state does not
change what ultimately gates a merge; it only defers the heavy lanes to the
un-draft moment.

## Required reviews

> **Solo-maintainer reality (read this first).** sparq is a **single-maintainer,
> agent-driven** repository: every PR is authored by `@jeswr` or by an automated SPARQL
> agent acting on his behalf. GitHub does **not** let an author approve their own PR, so a
> *human-approval* requirement (`required_approving_review_count ≥ 1` and/or
> `require_code_owner_review`) would **deadlock** the merge train — there is no second human
> to approve. The **live ruleset therefore sets `required_approving_review_count: 0` and
> `require_code_owner_review: false` deliberately**, and substitutes a *bot/automated*
> review layer (Copilot code review on push + CodeQL code-scanning gate + the `ci-summary`
> aggregator + conversation-resolution) for the missing second human. This is the same
> reality OpenSSF Scorecard's `Code-Review` / `Branch-Protection` checks score down — see
> [§Solo-maintainer & the Scorecard score](#solo-maintainer--the-scorecard-code-review--branch-protection-score)
> below; the settings here are written to match **what is actually enforced**, not an
> aspirational two-human flow the repo cannot run.

- **Approving reviews — `0` required (deliberate, solo-maintainer).** The live ruleset's
  `pull_request` rule sets `required_approving_review_count: 0` and
  `require_code_owner_review: false`. [`CODEOWNERS`](../CODEOWNERS) still records ownership
  of the high-risk paths (`sparq-zk*`, `sparq-mpc`, `sparq-core`, `sparq-server`,
  `.github/`, `deny.toml`, `SECURITY.md`) so that *if/when* a second trusted reviewer is
  added, code-owner review can be flipped on without re-deriving who owns what; today it
  documents intent rather than gating.
- **Stale-approval dismissal — `false` (no human approvals to dismiss).** With zero required
  human approvals there is nothing to stale-dismiss; the live ruleset sets
  `dismiss_stale_reviews_on_push: false` to match. (Copilot review *does* re-run on push —
  `review_on_push: true`.)
- **Require the automated code review** (GitHub Copilot code review + the CodeQL
  code-scanning review). The live ruleset enables Copilot code review on push
  (`copilot_code_review` rule, `review_on_push: true`) and treats **CodeQL code-scanning
  alerts** as blocking via the `code_scanning` rule (`CodeQL`, `alerts_threshold:
  errors_and_warnings`, `security_alerts_threshold: all`). The CodeQL run is also aggregated
  by `ci-summary` as the `CodeQL analysis (rust)` check-run; the code-scanning *results*
  rule is the complementary alert-severity gate.
- **Require conversation resolution before merging** — all PR review threads (human and
  bot, incl. Copilot/CodeQL) must be resolved (live ruleset `pull_request`
  `required_review_thread_resolution: true`). (Also listed under "Other settings".)
- **Code-quality rule active.** The live ruleset also carries a `code_quality` rule
  (`severity: all`), GitHub's built-in PR quality signal, alongside the checks above.

## History and push rules

- **Require linear history** — merges to `main` must not introduce merge commits. The live
  ruleset enforces this by allowing **only the squash merge method**
  (`pull_request.allowed_merge_methods: ["squash"]`) and a `non_fast_forward` rule, which
  matches the "gate and merge one branch at a time" discipline in `AGENTS.md`.
- **Block force pushes** to `main` (live ruleset `non_fast_forward` rule).
- **Block branch deletion** for `main` (live ruleset `deletion` rule).

## Other settings

- **Repository administrators can always bypass the ruleset.** The live ruleset's
  `bypass_actors` list contains the repository-role actor (`actor_id: 5`,
  `bypass_mode: always`). This is an explicit exception to uniform enforcement, not a
  compensating control. The automated landing flow does not use the bypass: auto-merge
  still enters the merge queue and waits for its required checks.
- **Use the merge queue.** The live `merge_queue` rule groups with `ALLGREEN`, admits at
  most 8 entries per merge, and gives required checks 60 minutes to report
  (`check_response_timeout_minutes: 60`). Its **throughput** parameters — how many
  entries the queue speculatively builds, and the minimum-group-size wait — are recorded
  in *Merge-queue throughput settings* below.
- **Require conversation resolution before merging** (all PR review threads resolved —
  `required_review_thread_resolution: true`).

### Merge-queue throughput settings (sq-6vshe.16)

<!-- [OPUS-5] sq-6vshe.16 / issue #2759 — LEVER 3 of
     research/ci-mergequeue-speedup-2026-07.md §3.3. Records the throughput parameter
     set, the min-entries-wait audit VERDICT, and the CodeQL merge_group placement
     re-verdict. INVARIANT: no required-check change — the sole required context stays
     `gate`; ALLGREEN grouping and squash-only are untouched by all three items. -->

The `merge_queue` rule's throughput parameters:

| Parameter | Value | Effect |
|---|---|---|
| `max_entries_to_build` | `3` | entries built speculatively **in parallel** — the queue's drain capacity |
| `max_entries_to_merge` | `8` | entries merged in one group (the cap the omnibus batcher folds overflow past) |
| `min_entries_to_merge` | `1` | one queued entry is enough to form a group |
| `min_entries_to_merge_wait_minutes` | `5` | **inert** at `min_entries_to_merge: 1` — see (b) |
| `grouping_strategy` | `ALLGREEN` | one red leg requeues the whole entry |
| `check_response_timeout_minutes` | `60` | required-check reporting deadline |

Provenance: `max_entries_to_merge`, `grouping_strategy` and
`check_response_timeout_minutes` are in the verified live-ruleset table at the end of
this document. The other three are read from the 2026-07-10 profile of ruleset
`17688455` (`research/ci-mergequeue-speedup-2026-07.md` §1, §3.3, §6) and are **not**
re-verified against the live API at this commit — this is a doc-only change. Confirm
them with the `gh api …/rulesets/<id>` recipe below before acting on (a).

**(a) `max_entries_to_build` 3 → 5 — approved in principle, NOT yet requested; blocked
on `sq-6vshe.14`.** Drain capacity scales directly with this number: a full 8-deep queue
takes `ceil(8/3) = 3` entry-wall waves at 3 and `ceil(8/5) = 2` at 5 — a third off a
full-depth drain — and the entry-failure rate measured over the profile window (**0
failed of the last 250 `merge_group` runs**) means the extra speculative builds are
almost never discarded work. But it is **conditional, and the condition is not yet met**:
5 concurrent entries × ~30 jobs needs the runner-pool headroom that the redundant
push-to-main waves currently burn, and the same profile measured **43–225 s of per-job
runner-queue delay during a 3-entry burst**. Raising parallelism *before* the push-skip
lands would worsen that contention, not relieve it. `sq-6vshe.14` (the `queue-validated`
push-skip, §3.1 of the design record) has **not** landed — no such job exists anywhere in
`.github/workflows/` as of this commit — so the ruleset edit stays **unrequested**.
Agents cannot edit rulesets, so when the precondition clears the edit is carried as a
maintainer steer issue, not applied from a PR.

**(b) `min_entries_to_merge_wait_minutes: 5` — AUDITED; verdict INERT, no edit needed.**
The concern was that this is a flat ~5 minute tax on every quiet-period (single-entry)
merge. It is not. The field is a **ceiling on waiting, not a floor**: per the rulesets
API it is the time the queue waits *after the first entry is queued, while the minimum
entry count is **not** met*, before going ahead and merging anyway. With
`min_entries_to_merge: 1` that minimum is satisfied by the first entry itself, so the
wait condition is never entered and the timer never binds. This is corroborated by the
measured entry-wall decomposition, which `ceil(position / capacity) × entry-wall` plus
the async ruleset evals accounts for with no residual flat term. The verdict rests on
that field semantics plus the decomposition, **not** on a direct measurement: nobody has
diffed the enqueue→merge timestamps of an isolated quiet-period single-entry merge. That
observation would settle it outright and is the cheap confirmation to run if a
quiet-period merge ever *looks* like it stalled ~5 m before merging. Setting it to `0` would
be a cosmetic no-op and is therefore **not** requested — but the pairing is load-bearing:
if `min_entries_to_merge` is ever raised above 1 this value becomes live, and the ~5 m
quiet-period tax becomes real and must be re-audited then.

**(c) CodeQL on the queue-blocking path — re-verdict KEEP; measured non-pole.**
Re-confirming `sq-6vshe.6` / issue #1815 with `merge_group`-specific data: over the
profile window CodeQL ran **3.8 m median / 4.2 m max (n ≈ 19)** on the merge-queue ref,
against a 15.1 m median entry wall set by `CI` (14.4 m median). It is buildless and it is
not the pole — the 20–40 minute figure that originally motivated moving it predates the
buildless migration. Moving a security gate off the queue-blocking path to save ≈ 0 is
all risk and no win: **REJECTED**, and the alerts-at-zero posture plus the ruleset's
`code_scanning` rule stay intact. **Read this against the 2026-07-18 operational
disable** (*Merge-queue subset* above): the conclusion stands, but its premise no longer
describes today's CI — `codeql.yml` is `disabled_manually`, so no CodeQL check-run is
produced on any event and it costs the queue nothing at all right now. The standing
meaning is forward-looking: when PR #3427 settles the successor policy and CodeQL runs
again, **queue latency is not a valid argument for keeping it off the blocking path** —
that premise was measured and falsified.

## How this maps to the merge discipline

`AGENTS.md` defines the landing gate as *full-workspace clippy + `cargo test` + the
conformance/perf/coverage ratchets, all green*, with parallel worktrees gated and merged
**one branch at a time**. The single required check — `ci-summary / gate` — is the CI
enforcement of that gate: it aggregates every other check-run, so the gate stays complete
even as jobs are added or renamed. Linear history (squash-only) + the automated review
layer (Copilot + CodeQL code-scanning) + conversation resolution enforce the one-at-a-time
merge discipline — human approvals are **not** required (solo-maintainer; see
[§Solo-maintainer & the Scorecard score](#solo-maintainer--the-scorecard-code-review--branch-protection-score)).
When a new ratchet or gate is added to a CI workflow it is covered
automatically (no ruleset edit needed); update the informational table above so reviewers
keep an accurate map.

### Omnibus batching (merge-queue overflow)

The merge queue on `main` drains individually-armed worker PRs up to its per-window cap
(`max_entries_to_merge: 8`). When **more than 8** reviewed worker PRs (open `sparq-agent/*`
heads carrying `review:pass` with an active auto-merge arm by `app/sparq-orchestrator`)
are waiting at once, the scheduled/event-driven batcher
([`scripts/batch-merge.py`](../scripts/batch-merge.py), run by
[`.github/workflows/batch-merge.yml`](../.github/workflows/batch-merge.yml), every
15 minutes) folds the overflow — everything beyond the 8 lowest-numbered PRs — into one
`sparq-omnibus/<class>-<utcstamp>` integration PR **per change-class** (issue #3433:
`slim` = constituents whose diffs are docs-/orchestration-only per `ci_select`'s audited
allowlist, so a slim batch rides the slim merge-group lane set and never waits on a
full-matrix run; `engine` = everything else, fail-closed; at least 2 and at most 15
constituents per omnibus — batch-15 per 15-minute run tracks the 60-merges/hour target
and keeps a v2 culprit bisect at log2(15) ≈ 4 runs; one omnibus per class in flight).
Each omnibus is fresh off `main`, built with sequential `--no-ff` merges (a conflicting
constituent is skipped and stays individually armed) and armed so a single
queue slot lands the whole batch; the omnibus body carries `Closes #` refs for every
constituent's issue, and once it merges the batcher closes each contained constituent PR.
The `sparq-omnibus/` prefix (and the absence of any `review:*` label) keeps these PRs out
of the registry's worker-review enumeration, which only admits `sparq-agent/issue-<n>-…`
heads. The omnibus branch/PR is pushed, created and armed with a **sparq-orchestrator App
installation token** (repo secrets `ORCHESTRATOR_APP_ID` / `ORCHESTRATOR_APP_PRIVATE_KEY`):
a `GITHUB_TOKEN`-created PR gets its workflow events suppressed, so the required
`ci-summary / gate` would never report on its head and the merge queue would never admit
it (admission requires the required checks to pass *before* entry). Without those secrets
the batcher fail-softs to hygiene-only mode (no new omnibus is created). Failure handling
is liveness-bounded: an omnibus whose head `gate` concluded in failure or that conflicts
with `main` is closed and its branch deleted; a young mergeable omnibus whose auto-merge
arm was dropped (merge groups drop the arm on a failed group) is re-armed idempotently;
and an omnibus still unmerged past the age bound (`MAX_OMNIBUS_AGE_HOURS` in the script —
the backstop for merge-group failures, which report on the queue's synthetic ref, not the
PR head) is closed so it can never suppress future batching. In every failure case the
constituents remain individually armed, so the failure mode is "no worse than unbatched"
(bisection is a tracked v2). The same workflow's `ring` job fires on every push to `main`
and, when the `REGISTRY_RING_TOKEN` secret is configured, pokes the
`jeswr/agent-account-registry` dispatcher so freed capacity is picked up immediately
(fail-soft: without the secret it skips with a notice and the registry cron is the
backstop). The batcher is **not** a required check and never runs on a PR head commit.

> All third-party GitHub Actions across `.github/workflows/*.yml` are **pinned by full
> commit SHA** (with a trailing `# vX.Y.Z` comment that Dependabot follows), resolving
> the Scorecard `Pinned-Dependencies` alerts. The one documented nuance:
> `dtolnay/rust-toolchain` is pinned to the **commit SHA of its `stable` / `1.88`
> branch tip** — that action selects the toolchain from the `action.yml` content at the
> ref (input default `stable`, or a hard-wired `1.88.0`), not from the ref *name*, so
> the SHA pin preserves toolchain selection (verified against the action source).

## Solo-maintainer & the Scorecard Code-Review / Branch-Protection score

<!-- [OPUS-4.8] Solo-maintainer evidence for OpenSSF Scorecard Code-Review /
     Branch-Protection (bead sq-sto1, gap GX-OSSF-3). -->

This section is the **doc-of-record evidence** for why OpenSSF Scorecard's `Code-Review`
and `Branch-Protection` checks score below 10 for this repository, and what *compensating*
controls stand in. It is the in-repo half of gap **GX-OSSF-3**
([`compliance/openssf/gap-register.md`](../compliance/openssf/gap-register.md)); the
remaining half is the maintainer periodically re-confirming the **live** ruleset against
this document (procedure below).

### Why the score is depressed (honest, not a defect)

- **`Code-Review`** — Scorecard infers code review from **merged-PR history** and
  **discounts self-approval**. In a single-maintainer, agent-driven repo there is no second
  human to record an independent approving review, so the history-derived signal is weak by
  construction. The repo does **not** fake this with a self-approval (which Scorecard
  discounts anyway and which the [`AGENTS.md`](../AGENTS.md) honesty posture forbids).
- **`Branch-Protection`** — Scorecard rewards *classic*-branch-protection settings such as
  `required_approving_review_count ≥ 1`, `require_code_owner_review`, and
  stale-review-dismissal. The live model deliberately sets all three to the
  "no second human" values (`0` / `false` / `false`, see [§Required reviews](#required-reviews)),
  so those particular sub-signals do not earn points even though the *substantive*
  protections (no force-push, no deletion, squash-only linear history, conversation
  resolution, CodeQL alert gate, a required CI aggregator, and merge-queue admission) are
  present and enforced for the normal automated landing path. Repository administrators
  retain the always-on bypass documented above.

These are **inherent to the operating model**, not fixable code changes — consistent with
the disposition recorded in `compliance/openssf/gap-register.md` (the Scorecard SARIF is no
longer uploaded to code-scanning precisely because these are posture *scores*, not code
alerts).

### Compensating controls (what substitutes for the missing second human)

| Missing classic signal | Compensating control (live + enforced) |
|---|---|
| Independent human approving review | **GitHub Copilot code review on every PR** (`copilot_code_review`, `review_on_push: true`) — an automated, independent reviewer recorded on the PR. |
| Code-owner gate | **CodeQL code-scanning gate** (`code_scanning` rule, `CodeQL`, `errors_and_warnings`) — blocks merge on new alerts; plus the SHA-pinned clippy/test/conformance gate aggregated by `ci-summary`. |
| Review-thread accountability | **Conversation resolution required** (`required_review_thread_resolution: true`) — every Copilot/CodeQL thread must be resolved before merge. |
| "Trusted committer only" | **No equivalent ruleset control.** Repository administrators can always bypass (`RepositoryRole`, `actor_id: 5`); the normal automated flow does not bypass and remains constrained by **merge-queue admission**, **squash-only** merges, **no force-push**, and **no deletion**. |

The agent operating discipline (`AGENTS.md`) adds a *process* layer on top: changes land via
PR (never direct push), and an out-of-band Codex/roborev review pass is run before arming a
PR for merge. That review is not visible to Scorecard's history heuristic, but it is the
real independent-review substitute in practice.

### Verifying the live ruleset matches this document

The live ruleset is configured **out-of-repo** and cannot be asserted from a tracked file,
so confirm it with the GitHub API (read-only token is sufficient):

```sh
# List rulesets on the default branch and grab the `main` ruleset id.
gh api repos/sparq-org/sparq/rulesets

# Dump the full rule set and eyeball it against this document.
gh api repos/sparq-org/sparq/rulesets/<id> | python3 -m json.tool
```

As verified on the date of this commit, the live `main` ruleset
(`enforcement: active`) carries one always-on repository-administrator bypass actor
(`actor_id: 5`, `actor_type: RepositoryRole`) and exactly these rules, all of which match
the sections above:

| Live rule (`type`) | Key parameters | Doc section |
|---|---|---|
| `deletion` | — | History and push rules |
| `non_fast_forward` | — | History and push rules (force-push + linear history) |
| `pull_request` | `required_approving_review_count: 0`, `require_code_owner_review: false`, `dismiss_stale_reviews_on_push: false`, `required_review_thread_resolution: true`, `allowed_merge_methods: ["squash"]` | Required reviews |
| `required_status_checks` | one context `gate`, `strict_required_status_checks_policy: false` | Required status checks |
| `code_quality` | `severity: all` | Required reviews |
| `code_scanning` | `CodeQL`, `alerts_threshold: errors_and_warnings`, `security_alerts_threshold: all` | Required reviews |
| `copilot_code_review` | `review_on_push: true`, `review_draft_pull_requests: false` | Required reviews |
| `merge_queue` | `grouping_strategy: ALLGREEN`, `max_entries_to_merge: 8`, `check_response_timeout_minutes: 60` | Other settings; Merge-queue throughput settings; Omnibus batching |

The `Key parameters` column is a selection, not an exhaustive dump: the `merge_queue`
row's remaining throughput parameters (`max_entries_to_build`, `min_entries_to_merge`,
`min_entries_to_merge_wait_minutes`) are recorded in *Merge-queue throughput settings*
above, sourced from the 2026-07-10 profile snapshot rather than re-verified here — fold
them into this row the next time the ruleset is dumped and verified.

If a future check finds drift (e.g. a rule added or a parameter changed), update **this
table and the matching section above in the same commit** so the doc-of-record never lags
the live ruleset.
