# Merge-queue critical path: measured profile + five speedup levers (2026-07)

> 🤖 SPARQ agent [FABLE-5] — maintainer-commissioned design record under epic **sq-6vshe**
> (CI structural speedup program). DESIGN ONLY: no workflow edits in this PR. All numbers
> are a dated snapshot of GitHub-hosted-runner CI runs (2026-07-10, run ids cited) —
> steering data for this program, not canonical performance claims.

## 1. What the maintainer experiences vs what the queue does

Observed symptom: "~40 min+ per merge group". The measured decomposition is:

```text
PR time-in-queue ≈ ceil(position / capacity) × entry-wall  +  post-gate async ruleset evals
                    capacity   = max_entries_to_build = 3      (ruleset 17688455)
                    entry-wall = ci-summary gate duration      (median 15.1 m, p90 23.4 m)
                    async evals: code_quality + code_scanning rules, API-invisible,
                                 observed ~11 min+ on PRs (reference-sparq-merge-mechanics)
```

A PR at position 4–6 in a drain therefore waits ≈ 2 × (15–23 m) + evals ≈ **40–60 min even
when every check is green and nothing fails**. The queue is not churning: of the last 250
`merge_group` workflow runs, **238 succeeded, 0 failed, 0 cancelled** (12 in-flight at query
time). Restart churn (the ALLGREEN requeue cascade) is a real historical failure mode
(2026-07-08 wasm-drift episode; 2026-07-02 congestion collapse) but is NOT the current
steady-state cost. The steady-state cost is **entry-wall × capacity-3 serialization**.

## 2. Measured profile (2026-07-10)

### 2.1 Per-workflow entry wall on `merge_group` (n≈17–20 successful runs each, last 250)

> **STALE AS A BASELINE — do not diff post-2026-07 numbers against these durations.** [OPUS-5]
> The 2026-08-01 re-sample under §3.4a below measured the ci-summary entry wall at a
> median well ABOVE the 15.1 m recorded here. That regression was already present on
> 2026-07-26 — so it predates the sq-6vshe.17 demotion and is not caused by it; this
> record does not establish which change between 07-10 and 07-26 caused it. The table
> remains as the dated
> 2026-07-10 snapshot that motivated the levers; the operative baseline is now the
> pre-cutover window in the §3.4a measurement note.
>
> **LANE SET RE-VERIFIED 2026-08-01 (issue #5165) — it was stale in BOTH directions.**
> [OPUS-5] Six of the original twelve lanes have LEFT the queue; three lanes that gate
> there today were never in the table. The split below is verified STRUCTURALLY, from
> the `on:` blocks of every `.github/workflows/*.yml` in this checkout — not from run
> history. **No lane duration here has been re-sampled**: the medians are all still the
> 2026-07-10 figures, retained only to say which lanes were poles *then*. Re-sampling
> per-lane medians for the current set is the open residual in §5.

**(a) Lanes that still trigger on `merge_group`.** Median column = the 2026-07-10
snapshot, kept for the pole ranking it established, NOT as a current figure.

| workflow (check family) | gating check-run(s) on the queue ref | 2026-07-10 median | status |
|---|---|---|---|
| ci-summary | `gate` | 15.1 m | IS the gate — duration ≈ entry critical path. **Superseded by §3.4a** (20.2 m median / 24.6 m p90, post-2026-07-30) |
| **CI** (ci.yml) | ~30 jobs, event- and selection-gated | **14.4 m** | **the pole. Superseded by §3.4a** (19.6 m median / 24.0 m p90 on heavy entries) |
| feature-matrix | `opt-in group (…)`, `feature-matrix check-tier (…)`, `pre-merge C1 (…)`, the three no-default-features legs, `fedclient dependency-boundary guard` | 5.8 m | #2 pole on engine-touching entries — not re-sampled |
| vectorized-feature-off | `vectorized-feature-off quick-gates (…)`, `artifact-exact-equality (wasm bundle feature-OFF)` | — | **never sampled** (absent from the original table). `artifact-exact-equality` builds the feature-OFF wasm bundle TWICE in one run, so it is a plausible unmeasured pole on wasm-touching entries |
| docs-quality | `docs-quality quick-gates` | 0.6 m | 13 jobs consolidated to 2 under sq-6vshe.20 — not re-sampled since |
| flow-on-gates | `flow-on-gates quick-gates (G1 + G2 + G6)` | 0.3 m | 4 jobs consolidated to 2 under sq-6vshe.20 — not re-sampled since |
| routing-self-tests | `Routing contract self-tests` | — | never sampled; hermetic, no build, no network |
| pr-area-label | `derive area labels` | — | postdates the snapshot; on the queue ref there is no pull-request context, so the deriver exits 0 in Python |
| codeql | **none** | 3.8 m | trigger set still lists `merge_group`, but the workflow is `disabled_manually` (2026-07-18), so it produces **no check-run on any event** and costs the queue nothing — see §3.3(c) and `docs/branch-protection.md` §*Merge-queue subset* |

**(b) Lanes that have LEFT the queue since the snapshot.** Each still runs and gates on
the PR head and re-runs on push-to-`main`; the accepted residual risk (a defect visible
only in a queued *combination*, caught post-merge and recovered by revert / fix-forward)
is recorded in `docs/branch-protection.md` §*Merge-queue subset*.

| workflow | 2026-07-10 median | `merge_group` removed by |
|---|---|---|
| Benchmarks (bench.yml) | 3.7 m | sq-6vshe.6 — lever 5, §3.5 |
| container-scan | 3.6 m | 2026-07-18 merge-queue-subset directive |
| fuzz (corpus-replay) | 0.8 m | 2026-07-18 merge-queue-subset directive |
| supply-chain | 0.7 m | 2026-07-18 merge-queue-subset directive |
| zk-toolchain | 0.2 m | 2026-07-18 merge-queue-subset directive |
| formal-verification | 0.2 m (max **22.7 m**) | 2026-07-18 merge-queue-subset directive |

**What the shrink does and does not explain.** The entry wall is `max` over lanes, not
`sum`: every departed lane was a measured NON-pole at the snapshot (largest median
3.7 m, against ci.yml's 14.4 m), so the six removals predict ≈**0** change in the median
wall. Their real value is runner-pool load and the TAIL — formal-verification's 22.7 m
change-coupled-Kani max was the only departed lane that could ever have *been* the pole,
and that tail is now off the queue. This is consistent with (and does not explain away)
§3.4a's finding that the median wall got WORSE over the same period: **the regression
cannot be attributed to the lane-set shrink**, and §3.4a independently locates the live
pole inside ci.yml's own build→test serial chain. Any re-sample should therefore be
scoped to ci.yml's job level, not spent re-timing the surviving sub-minute lanes.

### 2.2 Inside the CI workflow (job level; runs 29105265898 / 29105286547 / 29105790070)

The engine-touching worst case (run 29105265898, 18.9 m end-to-end) is a **serial chain**:

```text
select (~15 s + ~20 s queue)
  → build + archive test binaries   369–447 s
      steps: free-disk 72 s | compile+nextest-archive 162 s | artifact upload 90 s
             | doctests 18 s | checkout/toolchain/cache-restore ~25 s
  → slowest test shard (needs the archive): bulk 3/3 655 s   ← ends last
      (bulk 1/3 489 s, heavy-diskann 420 s, bulk 2/3 340 s, heavy-hnsw 228 s — imbalanced)
```

Running in parallel with that chain (they self-compile, start ~1 m in):

- coverage ratchet shards: 448–785 s each, last ends ~14.7 m in — **the pole whenever
  selection skips the test shards** (run 29105286547: all test shards selection-skipped,
  coverage shard 1/4 at 621 s WAS the critical path; entry wall 11.9 m).
- wasm build 265–289 s; inference conformance 182–205 s; docker smoke 168–201 s;
  clippy 154–158 s; the other conformance ratchets 44–76 s each.

feature-matrix on an engine-touching entry: ~15–20 selected opt-in engine legs at
320–397 s each, with **per-job runner-queue delays of 43–225 s** during a 3-entry burst
(run 29105286406, workflow wall 11.2 m). The queue delays are pool/provisioning
contention, not compute.

### 2.3 What is already fixed (do not re-litigate)

- **Selection IS live on `merge_group`** (sq-fmx4u.3/.4/.5, ci_select.py accepts
  `merge_group` as a diff-carrying event; evidence: run 29105286547 skipped every test
  shard, run 29105790070 ran a single coverage shard). Lever 4's "extend selection to the
  merge group" premise is largely DONE; what remains is §3.4 below.
- **CodeQL is buildless and fast** (3.8 m median). The 20–40 min figure predates the
  buildless migration (sq-6vshe.6, issue #1815). Lever 3's "CodeQL is the likely pole"
  premise is **falsified by measurement**.
- **rust-cache is wired per-job** across ci.yml + feature-matrix.yml (deps-only by
  design), and the test suite is build-once via `cargo nextest archive` (sq-vyxy).
  Merge-queue refs restore main-scoped caches (queue branches base on main).
- **Coverage is changed-cone** (sq-6vshe.8) and cross-runner-sharded (sq-piapk, #1871).
- **Per-PR fuzz is deterministic corpus-replay** (sq-6vshe.6, #1814).

### 2.4 The push-to-main shadow load (lever 1's target)

Every queue merge fast-forwards main and fires **~17 push-triggered workflows** on the
new tip — and `ci_select.py` treats `push` as mode=**full** (no PR diff), so this wave is
the FULL matrix: build+archive, all test shards, all coverage shards, all conformance,
the full feature-matrix leg set, randomized full-form fuzz, codeql, bench, container-scan,
… ≈ **200–400 runner-minutes per merged PR** (estimate from the §2.2 sums plus the
full feature-matrix). During a batch drain the *next* merge's push cancels the previous
wave mid-flight (observed: push CI for 2817c19c cancelled at 928 s, 6df875f6 at 311 s via
the per-ref concurrency group) — so most of that spend buys nothing, while its burst
competes with the ACTIVE merge-group entries (the measured 43–225 s per-job queue
delays, and the 2026-07-02 congestion-collapse tail risk where starved gate waiters
emitted false REDs). The org is enterprise-tier, so this is burst-provisioning latency
and waste rather than a hard concurrency cap — the delays are real wall-clock on the
critical path either way.

Key soundness fact for lever 1: with a merge queue, the SHA pushed to main **is** the
merge-group commit that just carried a full green check-suite. Post-merge push CI on that
SHA is validation-redundant by construction; what push runs uniquely provide today is
(a) side effects (bench canonical series, release-plz, pages deploy, scorecard),
(b) rust-cache priming on the main scope, (c) main's default-branch CodeQL alert state.

## 3. The five levers

### 3.1 Lever 1 — skip queue-validated re-validation on push to main → **bead sq-6vshe.14**

**Design.** A cheap `push`-event pre-job (`queue-validated`) queries the check-runs on
`github.sha`; if a successful `ci-summary / gate` produced by a `merge_group` event exists
on that exact SHA, pure-validation legs skip. **Fail-open**: no gate found (direct push,
revert, force-fix) → full run, exactly as today.

KEEP-list (never skipped): release-plz, pages, scorecard, the bench canonical-series
job (main-push writes the dashboard), randomized full-form fuzz (its placement IS
push+nightly per sq-6vshe.6), **codeql on push** (main's default-branch alert state is
fed by `refs/heads/main` analyses — the merge-group analysis lands on the queue ref;
keep until the association is verified, it costs ~4 m on one runner — **moot while
`codeql.yml` is `disabled_manually`; it produces no check-run on any event, so this
KEEP-list entry only binds if PR #3427 re-enables the workflow**), and ONE
**cache-primer** leg (build-archive or a deps-build) so the main-scope rust-cache stays
warm — note today's cache freshness already depends on whichever push wave survives
cancellation, so a small always-completing primer is a strict improvement.

> **KEEP-list addition — the COVERAGE legs, and this one is load-bearing, not a
> preference.** [OPUS-5] (issue #5149, added after §3.4a landed as sq-6vshe.17.)
> The instrumented coverage legs — `coverage-measure`, `coverage-engine-run`,
> `coverage-engine-merge` — must be **EXEMPT** from this skip. They are not re-validation
> on main: the MEASUREMENT was demoted off `merge_group` by sq-6vshe.17, and the *only*
> reason that demotion is sound is that push-to-main remains the enforcement point for the
> batch-stacking case (two PRs each ≥ floor merging to a tree < floor). Skip them here and
> the ratchet loses its last enforcement point silently — a strictly larger safety change
> than this lever's fail-open design claims to make. They are also off the queue's critical
> path by construction (that is what the demotion did), so exempting them costs this lever
> none of its estimated saving. `coverage-floors` is the cheap no-compile companion: under
> a minute, no instrumented build, and it is what keeps the `coverage` aggregate meaningful
> on main — keep it too; there is nothing to win by skipping it.
>
> **How to implement the exemption:** whatever `queue-validated` guard the pure-validation
> legs take, do NOT add it to those jobs — neither as an `if:` conjunct nor as a `needs:`
> entry (a `push`-event pre-job is conditional, and a skip propagates through `needs:` to a
> dependent that uses no status function, which these do not). Both shapes are pinned for
> the three measure legs by
> `scripts/tests/test_ci_select_wiring.py::TestCoverageMergeGroupDemotion` — its frozen
> `ALLOWED_UPSTREAM` REDs on either, so landing this lever over them fails a test rather
> than rotting silently; widen that set only with the exemption decided. Enforcement
> topology of record: `docs/branch-protection.md` §*Coverage MEASUREMENT off the merge
> queue*.

**Safety: SAFE** (fail-open guard + explicit keep-list + the nightly full-matrix
backstop and sq-va7at selection alarm are untouched).
**Est. saved:** ~200–400 runner-min per merged PR of pool load; −0.5–2 m median entry
wall via the measured queue delays; removes the congestion-collapse tail risk; and is
the **precondition** for raising queue parallelism (lever 3).

### 3.2 Lever 2 — caching within and across runs → **bead sq-6vshe.15** (extends sq-6vshe.5)

What exists is already the big win (per-job rust-cache for deps + build-once nextest
archive): the merge-group compile step is 162 s, not the naive multi-minute per-shard
rebuild. The honest remaining deltas, in measured order:

1. **Artifact diet** (on the critical path): the nextest archive upload is 90 s and every
   test shard re-downloads it. Tune zstd level / prune non-test artifacts / measure
   upload+download vs compile trade.
2. **`save-if` discipline**: rust-cache saves on `gh-readonly-queue/*` refs are dead on
   arrival (the ref is deleted post-merge; nothing can restore them) — restrict saves to
   `refs/heads/main`. Frees post-step seconds per job and cache-backend quota.
3. **sccache (GHA backend) A/B on build-archive** — measure-first, expectations LOW:
   deps are already warm via rust-cache; the changed crates of a PR always miss; only
   unchanged workspace crates hit, and feature-matrix legs key differently per cfg.
   Adopt only on a ≥60 s median win, else record the negative result.

**Safety: SAFE** (cache poisoning discipline per sq-6vshe.5 applies; measure-first).
**Est. saved:** −0.5–2 m entry wall, mostly from (1).

> **Status 2026-07-30 [SONNET-4.6] — sq-6vshe.15 landed items 1+2; item 3 is NOT done.**
>
> - **(1) partially done, and NOT the 90 s the profile attributed to compression.** The
>   `nextest-archive` upload now sets `compression-level: 0`, so upload-artifact stops
>   re-running its default DEFLATE-6 zip pass over an already-zstd-compressed
>   `nextest.tar.zst`. That removes re-compression CPU only; the 90 s step is CPU **plus**
>   transfer, and this change does not shrink the bytes, so the saving is un-quantified
>   until the next re-profile — no number is claimed here. The other two options the lever
>   listed (nextest `--zstd-level` retune, pruning non-test content from the archive) were
>   **NOT** attempted: both change the archive's bytes/content, so both need the
>   before/after test-count parity measurement this bead had no measured baseline for.
> - **(2) done, over a lane set much smaller than §2.1 implies.** `save-if: ${{ github.ref
>   == 'refs/heads/main' }}` is now on every `Swatinem/rust-cache` step in every workflow
>   that still triggers on `merge_group`. **That is only `ci.yml` (20 steps),
>   `feature-matrix.yml` (6, already compliant under sq-3sbrr) and
>   `vectorized-feature-off.yml` (1)** — most §2.1 lanes no longer trigger on
>   `merge_group` at all (`fuzz.yml`, `zk-toolchain.yml` and `formal-verification.yml`
>   under the 2026-07-18 maintainer directive; `bench.yml` under sq-6vshe.6), and the
>   nightly-only lanes asan/miri/kani never were queue-triggered. So this bead added the
>   guard to **21 steps**, not to the whole tree.
>   **Re-profile §2.1 before acting on any other lever here**: the record's own §6 caveat
>   ("re-profile if the lane set has materially changed") has now fired.
>   Pinned structurally by `scripts/tests/test_mergequeue_cache_posture.py`.
>   NEW COUPLING for **sq-6vshe.14**: main-scope cache freshness now depends entirely on
>   push-to-main runs, so .14 must keep its promised cache-primer leg.
> - **(3) NOT done — no verdict either way.** No sccache A/B was run, so there is neither
>   an adoption nor an honest negative result to record; the ≥60 s bar is untested. Nothing
>   about sccache is wired. Tracked as a follow-up issue.

> **Status 2026-09-01 [OPUS-5] — item 3: the INSTRUMENT is built; there is still NO
> VERDICT.** (issue #5164)
>
> Read the previous paragraph's verdict line as unchanged: **the ≥60 s bar remains
> untested and no number exists.** What landed is the measurement apparatus item 3
> needs and never had, not the measurement:
>
> - `.github/workflows/sccache-ab.yml` — a **`workflow_dispatch`-only** A/B over
>   ci.yml's `build + archive test binaries` compile step. One `prime` job populates an
>   sccache namespace, then 2 arms × 5 trials run on fresh runners: `control` is the
>   production configuration untouched, `treatment` differs **only** by `RUSTC_WRAPPER`.
>   Nothing in `ci.yml` changed; no production job gained a compiler wrapper.
> - `scripts/sccache_ab_verdict.py` — medians, the ≥60 s scoring, and the honesty
>   guards. It exits 0 for BOTH outcomes (a negative result is the deliverable, not a
>   failure) and exits 2 **INCONCLUSIVE** — refusing to emit any verdict — when the
>   instrument did not earn one.
> - `scripts/sccache-ab-namespace.sh` — the sq-6vshe.5 key schema
>   ({rustc version, host triple, feature-family, `Cargo.lock` digest}), so no key
>   aliases across toolchain / lockfile / feature-set, under an `sccache-ab-` prefix
>   that cannot collide with a future production sccache key.
> - `scripts/tests/test_sccache_ab_harness.py` — the harness's validity conditions,
>   pinned structurally and wired into `docs-quality.yml`.
>
> **Why the INCONCLUSIVE exit is the load-bearing part.** This record predicts a
> negative result, and "sccache did not help" and "sccache never actually ran" produce
> the *same* wall-clock table — they differ only in the sccache counters. A silently
> broken harness therefore yields exactly the answer everyone already expects, and it
> would be written here permanently. Two concrete instances of that failure were found
> while building this, by running the thing rather than reasoning about it:
> **(a)** sccache refuses to cache incremental compilation outright — with
> `CARGO_INCREMENTAL` non-zero every request lands in `not_cached: {"incremental": N}`
> with zero hits *and* zero misses, so the treatment arm silently degrades to
> "control plus wrapper overhead"; **(b)** the namespace script's first version piped
> `rustc -vV` straight into its digest, and because `set -euo pipefail` does not abort a
> failing command inside a `{ … } | sha256sum` substitution, a broken toolchain
> contributed an empty string and it still printed a confident key — aliasing across
> every rustc version, the one thing sq-6vshe.5 forbids. Both are now fail-closed.
>
> **What is still owed, and by whom.** Someone with dispatch rights must run the
> workflow on `main` and paste the verdict job's summary here — either an ADOPT (which
> then needs a real `ci.yml` change, separately reviewed) or the expected DO-NOT-ADOPT,
> at which point item 3 closes. **Do not record a negative result from an INCONCLUSIVE
> run.** Cost is ~11 workspace builds per dispatch, which is why it is dispatch-only and
> not scheduled.
>
> **Item 1's remaining halves are still NOT done** and are unchanged by this: the
> `nextest --zstd-level` retune and pruning non-test content from the archive both
> alter the archive's bytes, so both need the before/after test-count parity check and
> a decomposition of the upload step into compress-vs-transfer that no measurement here
> provides. This harness times the **compile** step only.

### 3.3 Lever 3 — prioritize/parallelize the thing about to merge → **bead sq-6vshe.16**

- **CodeQL: KEEP on the blocking path.** Measured 3.8 m median (buildless). Moving a
  security gate off the queue for ~0 saving is all risk, no win — the maintainer's
  suspicion was correct for the pre-buildless era, and is honestly falsified now. The
  alerts-at-zero posture + the ruleset's code_scanning rule stay intact. **REJECTED.**
- **Queue settings (maintainer ruleset edit, one-line each):**
  `max_entries_to_build` 3→5 lifts deep-queue drain capacity ×1.67; with the measured
  0/250 entry-failure rate the extra speculative builds are almost never wasted. Do this
  ONLY after lever 1 lands (5 concurrent entries × ~30 jobs each needs the pool headroom
  the push waves currently burn). Verify `min_entries_to_merge_wait_minutes: 5` is inert
  given `min_entries_to_merge: 1` — if it is delaying single-entry merges by up to 5 m,
  set it to 0 (a flat win on every quiet-period merge).
- **Test-shard rebalance** (the actual CI pole): bulk 3/3 at 655 s vs bulk 2/3 at 340 s
  is a ~2× imbalance on the serial chain's last hop. This is owned by the OPEN bead
  **sq-6vshe.7** (nextest partition rebalance) — annotated with this profile rather than
  re-beaded. Rebalance + one extra bulk shard ≈ −3–5 m off the engine-entry p90.

**Safety:** settings = SAFE-QUICK-WIN (conditional on lever 1); CodeQL move = REJECTED.
**Est. saved:** position-6 wait 2×E → 2×E with capacity 3→5 becomes ceil(6/5)=2 → same;
the gain appears from position >6 and during bursts (drain rate ×1.67). Plus up to 5 m
flat if the min-entries wait proves non-inert.

> **RESOLVED (sq-6vshe.16, issue #2759) — see `docs/branch-protection.md` §*Merge-queue
> throughput settings*, now the doc-of-record for all three items.** [OPUS-5]
> **(a)** `max_entries_to_build` 3→5 approved in principle but **NOT requested**: lever 1
> (`sq-6vshe.14`) has not landed — no `queue-validated` push-skip job exists in
> `.github/workflows/` — so the pool-headroom precondition is unmet.
> **(b)** the `min_entries_to_merge_wait_minutes: 5` audit **closes as INERT**: the field
> is a ceiling on waiting while `min_entries_to_merge` is unmet, not a floor, so at
> `min_entries_to_merge: 1` it never binds. The "up to 5 m flat" line above is therefore
> **not** a real saving — no edit needed (re-audit if `min_entries_to_merge` ever rises
> above 1).
> **(c)** the CodeQL KEEP verdict stands on the measurement but has been **overtaken by
> events**: `codeql.yml` was operationally disabled on 2026-07-18, so it produces no
> check-run on any event and costs the queue nothing today. Its forward-looking meaning
> is that queue latency is not a valid argument against re-enabling it on the blocking
> path (PR #3427 owns the successor policy).

### 3.4 Lever 4 — skip tests for unaffected crates in the merge group

Change-based selection ALREADY runs on `merge_group` with the sound fail-closed rule set
(fmx4u: skipped ⇒ provably outside the reverse-dep closure; anything ambiguous ⇒ full;
gate REDs if a skip's `select` verdict is missing/unsuccessful). Two real remainders:

**(a) Coverage off the merge-group blocking path → bead sq-6vshe.17 — NEEDS-CAREFUL-DESIGN.**
Coverage shards (448–785 s) are the entry pole whenever selection skips the test shards.
Coverage is a RATCHET, not a correctness test: a floor regression that slips through a
batch is detectable and recoverable post-merge, unlike a functional bug — which is what
makes demotion designable at all. Proposed enforcement topology: PR coverage unchanged
(the primary gate); `merge_group` drops the coverage-measure legs; the **push-to-main
run keeps ONLY its coverage legs** (exempt from the lever-1 skip — post-merge, off the
queue's critical path); a main coverage red auto-files a P1 (mirror sq-va7at / the
sq-6vshe.6 demotion auto-bead protocol) and blocks further ratchet advances until green.
The residual risk is the batch-stacking case: two PRs individually ≥ floor merging to
< floor — rare, caught ~15 m later on main, recoverable, floor never silently lowered.
This needs an explicit maintainer-visible design (proceed-and-document), not a quick flip.
**Est. saved:** −2–6 m median entry wall (run 29105286547 would have been ~7 m, not 11.9 m).

> **IMPLEMENTED 2026-07-29 (bead sq-6vshe.17), as proposed above.** The
> maintainer-visible design is recorded in `docs/branch-protection.md` §*Coverage
> MEASUREMENT off the merge queue* (the proceed-and-document requirement); the wiring is
> in `ci.yml` and pinned behaviourally by
> `scripts/tests/test_ci_select_wiring.py::TestCoverageMergeGroupDemotion`.
> Deltas from the sketch above, all narrowing:
> * the fast **no-compile** floor gates (`coverage-floors`: test-presence, floor
>   MONOTONICITY, shard-partition) were **kept on `merge_group`** — they are well under a
>   minute and they are what makes "no committed floor is silently lowered" true of a
>   *batch*, not just of a PR. Only the instrumented MEASURE legs were demoted.
> * the push-to-main run is left **unchanged** rather than narrowed to "only its coverage
>   legs": lever 1 (sq-6vshe.14) is not implemented yet, so there is no push-run skip to
>   be exempt from. The exemption is instead pinned as a **test** that REDs if a future
>   push-run skip covers coverage — the coordination point, made mechanical.
> * "blocks further ratchet advances until green" landed as
>   `coverage-gate.py --check-advance-allowed`: it blocks only a **raise** of an existing
>   floor while the `[demoted-lane] lane=coverage-ratchet-main` alarm is open, never the
>   recovery path (a governed lowering under `--allow-lower`), never a new crate row, and
>   it fails **open** if the alarm probe is unavailable.
>
> **MEASURED 2026-08-01 (issue #5147) — the −2–6 m projection lands at the BOTTOM of its
> band; the saving is real but the absolute wall is WORSE than §2.1.** [OPUS-5]
> Cutover = commit `3ab83df` (PR #5146, 2026-07-30T22:22:32Z), the commit that introduced
> the `github.event_name != 'merge_group'` guard. Sample = successful `merge_group`
> ci-summary runs split on `run_started_at`, duration = `updated_at − run_started_at`
> (the §6 method): **n=94 post** (n=80 after excluding the sub-8-minute trivial-entry mode
> — see below), against the n≥15 the issue asked for.
>
> | window (successful `merge_group` ci-summary) | n | median | p90 |
> |---|---|---|---|
> | PRE, matched 40.9 h immediately before cutover | 31 | 23.7 m | 27.6 m |
> | PRE, broader 2026-07-26…07-28 baseline | 146 | 21.7 m | 27.4 m |
> | **POST, cutover → 2026-08-01T15:15Z** | **80** | **20.2 m** | **24.6 m** |
>
> (Entries ≥8 m only. The raw distribution is bimodal — a ~2–4 m mode carrying 9–15 % of
> every window, consistent with entries whose change-class skips the Rust matrix, though
> the job level of that mode was not inspected — so an all-entries median tracks
> window composition as much as it tracks CI. All-entries figures move the same way:
> 23.2 → 19.4 m median, 27.6 → 24.6 m p90.)
>
> **Verdict: CONFIRMED, at the low end.** Median saving **−1.5 m** against the broader
> pre baseline and **−3.5 m** against the matched-window one; p90 **−2.8 to −3.0 m**.
> The honest range is **−1.5 to −3.5 m median**, i.e. the bottom of the projected −2–6 m,
> and below it on the more conservative baseline. Corroborated independently on `ci.yml`'s
> own wall (heavy entries: median 22.4 → 19.6 m, p90 27.5 → 24.0 m). Not a quiet-period
> artifact: the post window carried ~2.8 times the pre window's entry rate (0.83 → 2.30
> entries/h), so it measured *more* pool contention, not less.
>
> **Mechanism verified**, not just inferred — on post-cutover run `30652403619` every
> instrumented coverage MEASURE leg reports `skipped` while `coverage floors` and
> `coverage ratchet + test-presence gate` report `success`, exactly the topology the
> guard above specifies.
>
> **Two corrections this sample forces:**
> 1. **§2.1's 15.1 m median / 23.4 m p90 is no longer the baseline.** The wall had already
>    regressed to ~21.7 m median by 2026-07-26, *before* this bead. So the post-change
>    ~20 m median is BETTER than the run this demotion inherited and WORSE than the
>    2026-07-10 snapshot; reading it against §2.1 would falsely score this bead as a
>    regression. The §6 re-profile caveat has fired again.
> 2. **The next pole is the build→test serial chain** — the branch issue #5147 named for a
>    saving at the low end of the band. On run `30652403619` (26.5 m wall) the chain is
>    build+archive 9.1 m → slowest shard `bulk 1/3` 16.6 m ≈ 25.7 m serial, essentially the
>    entire wall, with the coverage legs no longer even candidates. Both hops are well past
>    §2.2 (build+archive was 369–447 s; slowest shard was 655 s), and the imbalance §3.3
>    flagged persists with the slow shard's identity moved (`bulk 1/3` 16.6 m vs `bulk 2/3`
>    8.7 m). **→ lever sq-6vshe.7 (shard rebalance), not further demotion.**
>    *Caveat: this hop decomposition is ONE post-cutover run, not a distribution — it is
>    enough to locate the pole, NOT enough to attribute the (1) regression to these hops.
>    A job-level pre/post sample is the follow-up.*
>
> Method: unauthenticated `GET /repos/sparq-org/sparq/actions/workflows/{ci-summary.yml,
> ci.yml}/runs?event=merge_group&status=success` (300 + 200 runs) and
> `GET /actions/runs/30652403619/jobs`.

**(b) Selection-soundness memo + the fmx4u §7 P8 decision → bead sq-6vshe.18 — SAFE.**
The union-diff-vs-target-tip argument that makes merge-group selection sound under
ALLGREEN + a sole required `gate` is currently a bead note, and the maintainer's
stricter-rule option (`event==merge_group ⇒ mode=full` — a one-line selector change that
would REVERSE much of this lever) is still an open decision. Codify the argument in
research/change-based-test-selection.md, present the decision, recommend KEEP-selected
(nightly full backstop + sq-va7at alarm already fence it).

Not worth extending selection to: conformance ratchets (44–76 s each, parallel,
never the pole). The other two candidates this paragraph once listed are **moot on the
queue as of the §2.1 re-verification**: container-scan no longer triggers on
`merge_group` (2026-07-18 directive) and codeql produces no check-run at all
(`disabled_manually`). Both remain non-candidates on the PR head for the reasons given —
container-scan is parallel and off the pole, codeql language-scoping is a small win
against a security gate the `code_scanning` ruleset expects analyses from.

### 3.5 Lever 5 — benchmarks → nightly EC2 (sibling lane; cross-reference only)

The `Benchmarks` merge-group leg ("run + track benchmarks", 233 s + select) gated when
this was written. The sibling lane moving benchmark timing off shared runners to the
nightly EC2 lanes removes a 3.7 m median leg (rarely the pole) — but its REAL value is
retiring the last flaky-timing surface from the gate: with ALLGREEN grouping, one flaky
gating leg forces a whole-entry requeue (the worst churn multiplier), so the expected
saving is in the tail, not the median. No bead here — owned by the sibling; do not
double-implement.

> **The queue half of this is ALREADY DONE (§2.1 re-verification, 2026-08-01).** [OPUS-5]
> `bench.yml` dropped its `merge_group` trigger under sq-6vshe.6, so the `Benchmarks`
> leg no longer appears as a check-run on the queue ref and the flaky-timing requeue
> multiplier is already off the gate. The 3.7 m median leg it removed was a non-pole
> running in parallel, so the median-wall saving this lever implied is ≈0 — the tail
> claim above is the part that held. What the sibling lane still owns is the *placement*
> of the timing suites (nightly EC2 vs push-to-`main`); do not re-count the queue
> removal as a pending saving in the §4 ranking.

## 4. Ranking — (queue-time saved × safety)

| rank | lever | bead | verdict | est. saved | class |
|------|-------|------|---------|-----------|-------|
| 1 | push-to-main skip (validated SHAs) | sq-6vshe.14 | SAFE (fail-open) | 200–400 runner-min/merge; −0.5–2 m wall; collapse-tail removal; unlocks #3 | **SAFE-QUICK-WIN** |
| 2 | bench → nightly EC2 | (sibling lane) | SAFE as designed there | flake-requeue tail removed; the "−3.7 m leg" was a parallel non-pole, so ≈0 median (§3.5) | **queue half DONE** (no `merge_group` trigger since sq-6vshe.6); placement owned by the sibling |
| 3 | queue settings (build 3→5; min-wait audit) | sq-6vshe.16 | SAFE after #1 | drain ×1.67 deep-queue; the min-wait audit closed **inert** (§3.3 RESOLVED) so the "≤5 m flat" it once promised is **0** | **SAFE-QUICK-WIN** (maintainer ruleset edit, still blocked on #1) |
| 4 | coverage off merge_group | sq-6vshe.17 | LANDED; **measured** (§3.4a, 2026-08-01) | **−1.5–3.5 m median / −2.8–3.0 m p90 measured**, vs −2–6 m projected | **DONE** |
| 5 | test-shard rebalance | sq-6vshe.7 (existing, annotated) | SAFE | −3–5 m engine-entry p90 | existing bead — **now the top pole** (§3.4a) |
| 6 | cache/artifact diet + sccache A/B | sq-6vshe.15 | SAFE, measure-first | −0.5–2 m | SAFE |
| 7 | selection memo + P8 decision | sq-6vshe.18 | SAFE (docs/audit) | 0 direct; closes an open soundness decision | SAFE-QUICK-WIN |
| 8 | gate waiter off the runner slot | sq-6vshe.19 | SAFE but fiddly | frees 3–6 runner slots during drains | discovered, P3 |
| — | CodeQL off the blocking path | — | **REJECTED** — measured non-pole (3.8 m), security gate | ~0 | falsified premise |

**End-state estimate — 2026-07-10 projection, SUPERSEDED as an absolute target.** [OPUS-5]
It reads: (levers 1+3+4a+5 + the existing .7) median entry wall 15.1 m → ~9–12 m;
engine-entry p90 23.4 m → ~17–19 m (then bounded by the build→test chain, whose next
lever is the .7 rebalance and the closed-for-now .3/.4 engine-split reopening
conditions); a position-6 PR ≈ 40–60 m → ~15–25 m. The 2026-08-01 re-sample (§3.4a)
measures the post-.17 median at ~20 m, so this arithmetic starts from a base ~6 m too
low. Its *relative* deltas may still hold; its absolute targets do not. Re-derive after
sq-6vshe.7 lands.

## 5. Discovered work (beaded unless noted)

- sq-6vshe.19 — the ci-summary gate is a WAITER occupying a runner slot ~15–23 m per
  entry (×3–5 concurrent entries, + the push waiter). Its own doctrine (sq-90cv4) names
  moving it off the build-runner slot as the deferred deep fix.
- sq-6vshe.7 (existing) — **re-annotated 2026-08-01: now the top pole.** On post-.17 run
  `30652403619` the imbalance persists and has grown in absolute terms (`bulk 1/3` 16.6 m
  vs `bulk 2/3` 8.7 m), and build+archive → slowest shard is ~25.7 m of a 26.5 m wall.
  Originally annotated with the 2026-07-10 bulk-shard imbalance (655 s vs
  340 s) and the formal-verification change-coupled-Kani 22.7 m outlier (a per-leg
  wall-time item squarely in that bead's inventory scope).
- The `min_entries_to_merge_wait_minutes` semantics audit (folded into sq-6vshe.16).
- **Per-lane re-sample for the current queue lane set — OPEN (issue #5165).** The
  §2.1 lane SET is now verified against the checkout, but only two of its rows carry a
  2026-08-01 duration (ci-summary and CI, both from §3.4a); `feature-matrix`,
  `docs-quality` and `flow-on-gates` still carry 2026-07-10 medians taken before the
  sq-6vshe.20 job consolidation, and `vectorized-feature-off`, `routing-self-tests` and
  `pr-area-label` have **never** been sampled on the queue at all. Highest-value target
  is `vectorized-feature-off`'s `artifact-exact-equality` (two release-wasm builds per
  run — a candidate pole on wasm-touching entries that no measurement has ever covered);
  the sub-minute survivors are not worth a run-history pass. Per §2.1's `max`-not-`sum`
  argument, the *median-wall* re-profile that matters is the ci.yml JOB-LEVEL pre/post
  sample §3.4a already names as its own follow-up — these two should be one pass.

## 6. Method / reproducibility

`gh run list --event merge_group --limit 250` (duration stats over successful runs);
`gh api repos/sparq-org/sparq/actions/runs/<id>/jobs` for job/step decomposition (runs
29105265898, 29105286547, 29105790070, 29105286406); ruleset 17688455 via
`gh api repos/sparq-org/sparq/rulesets/17688455`; selection semantics from
scripts/ci_select.py + .github/workflows/ci-summary.yml; conclusions histogram over the
same 250-run window. Snapshot 2026-07-10 — re-profile before acting on a ranking if the
lane set has materially changed.

**Lane-set re-verification (2026-08-01, issue #5165) — a STRUCTURAL method, no run
history.** [OPUS-5] The §2.1 lane split was derived by parsing the top-level `on:` block
of every `.github/workflows/*.yml` and keeping the workflows that list `merge_group` as
a trigger, then reading each workflow's `jobs.*.name` for the check-run names that land
on the queue ref. Reproduce with `rg -l '^\s{2}merge_group:' .github/workflows/`, which
returned exactly the nine table-(a) workflows on 2026-08-01. (A YAML-parse variant also
works, but note that a YAML 1.1 loader such as PyYAML resolves the bare `on:` key to the
boolean `True`, so it must be looked up as `doc[True]`.) The split is pinned against
drift by `scripts/tests/test_mergequeue_lane_inventory.py`, which asserts
`docs/branch-protection.md` §*Merge-queue subset* names exactly the triggering set
(wired into the gating `docs-quality quick-gates` job). Two corrections this method contributes
that a run-history pass alone would miss: `codeql.yml` still *lists* `merge_group` but is
`disabled_manually`, so trigger presence ≠ a check-run; and `container-scan.yml` +
`supply-chain.yml` had also dropped the trigger under the 2026-07-18 directive, which the
`docs/branch-protection.md` §*Merge-queue subset* prose had not recorded (fixed in the
same change). What this method does NOT give is durations — see the §5 residual.
