# Getting the ci-summary gate off the build-runner slot — design record (sq-90cv4 follow-up)

> Status: **DESIGN-ONLY.** The shipped sq-90cv4 increment is the *adaptive saturation
> budget* in `scripts/ci_summary_gate.py` (unit-tested; see the PR that added this
> record). This record documents the **deferred deeper fix** — removing the gate's
> slot occupancy entirely — so the follow-up bead starts from a vetted design instead
> of re-deriving one against the merge-critical required check. Authored by Claude
> Fable 5 `[FABLE-5]`.

## 1. Problem

`ci-summary / gate` (the single REQUIRED branch-protection check) is a *waiter*: a
GitHub-hosted job that polls sibling check-runs until they are all terminal. While
polling it occupies one job slot in the account-wide hosted-runner concurrency pool.
Under load (many open PRs, each with its own gate) the waiters collectively starve
the build jobs they are waiting on; observed 2026-07-02 as a congestion collapse with
false `gate=FAILURE` verdicts on PRs whose real legs were green or merely queued.

The adaptive budget fixes the *false verdict* (queued-under-saturation now extends
the wait instead of concluding FAILURE; a genuine hang still fails). It does **not**
fix the *slot occupancy* — a saturated pool now holds gate slots longer, which is the
correct trade for verdict honesty but leaves throughput on the table.

## 2. Recommended deeper fix: event-driven evaluation, no resident waiter

Replace the resident poll loop with **short-lived evaluations triggered by sibling
completion events**:

- A `workflow_run` (`types: [completed]`) triggered workflow fires each time any
  sibling workflow finishes. Each firing runs for seconds: fetch the head commit's
  check-runs once, apply the *same* verdict function (`render_verdict` in
  `scripts/ci_summary_gate.py` — already extracted and unit-tested), and publish the
  result as a **commit status** (`statuses: write`) on the head SHA.
- Branch protection then requires that commit-status context instead of the
  `ci-summary / gate` job. The status is "pending" until the evaluator sees an
  all-terminal set, then success/failure by the existing semantics.
- Slot cost: O(seconds) per sibling completion instead of O(minutes-to-an-hour) per
  PR. No waiter exists to starve the pool, so the saturation false-RED class
  disappears structurally rather than being compensated for.

Why this shape and not the alternatives:

- **Concurrency-exempt / self-hosted tiny runner for the gate** — works, but needs
  org/runner configuration outside the repo (runner registration, labels, upkeep)
  and adds an always-on machine to the trust surface. Needs the maintainer; not a
  repo-only change.
- **Merge queue + org move** (already planned) reduces how many PR heads run CI
  concurrently but does not remove the waiter; complementary, not a substitute.
- **Keeping the poll loop but shortening it** regresses the verdict semantics the
  current design guarantees (waits for late-registering workflows, settle window).

## 3. Design risks the implementation bead must resolve

1. **Required-check migration.** Swapping the required check from a job to a status
   context must be atomic-ish: add the status context as required *alongside* the
   job first, prove parity on live PRs, then drop the job. A missing "expected"
   required check blocks every merge — stage it.
2. **Trust model changes (for the better, but verify).** `workflow_run` executes the
   workflow definition from the **default branch**, not the PR — the evaluator logic
   stops being PR-attested. Confirm the token scoping (`statuses: write` on the
   evaluator; the PR cannot forge the status).
3. **Bootstrap + quiet PRs.** A docs-only PR that triggers few/no sibling workflows
   needs at least one evaluation to publish the status (the current gate's
   stable-empty-set pass). A `pull_request`-triggered "seed" evaluation (short-lived,
   also non-resident) covers it, but its token cannot write statuses from forks —
   check the fork path.
4. **`merge_group` compatibility.** The merge-queue ref needs the same status; verify
   `workflow_run` events fire for merge-group-triggered sibling runs and that the
   status lands on the merge-group head SHA the ruleset checks.
5. **Event storms.** One evaluation per sibling-workflow completion is dozens per
   push; each is cheap, but debounce (concurrency-group per head SHA,
   `cancel-in-progress`) to keep it tidy.
6. **Startup race parity.** The current MIN_POLLS floor guards against verdicting a
   partial early set. The evaluator equivalent: never publish a success while any
   *expected* workflow for the trigger set has not yet registered — reuse the
   path-filter outputs or require a minimum sibling count seen before a green status.

## 4. Verdict-function reuse

The sq-90cv4 extraction means the deeper fix does **not** re-implement semantics:
`is_advisory` / `is_self` / `render_verdict` and their tests
(`scripts/tests/test_ci_summary_gate.py`) are the shared, already-gated brain. The
follow-up only replaces the *transport* (resident loop → event-driven invocations).

## 5. Status

**Stage 1 of §3.1 is implemented (bead sq-lfmvd); the required-check migration is
not.** `.github/workflows/ci-gate-status.yml` runs the §2 evaluator
(`scripts/ci_summary_gate.py --evaluate`, sharing `render_verdict` verbatim per §4)
and publishes the `ci-gate` commit status. It runs **in shadow**: the
branch-protection ruleset still requires only `ci-summary / gate`, and the poll loop
still runs, exactly as §3.1 demands. Stages 2 and 3 — adding `ci-gate` to the ruleset
alongside `gate`, then dropping `gate` and deleting the poll job — are **maintainer
edits to out-of-repo settings** and remain open. `docs/branch-protection.md`
§Slotless gate evaluation is the doc-of-record for the procedure and the stage-1 exit
criteria.

How the §3 risks stand:

| Risk | Status |
|---|---|
| 1. Required-check migration | **Resolved by construction** — staged, never swapped; a test reds if the `gate` job is deleted while the shadow lane is the only evaluator. Stages 2/3 are maintainer-owned. |
| 2. Trust model | **Resolved** — the evaluator runs the default-branch definition and checks out the default branch's script + registry; its token holds `statuses: write` only. The honest cost (a PR editing the gate script or registry is judged by `main`'s copy) is documented, not hidden. |
| 3. Bootstrap / quiet PRs / forks | **Resolved without a `pull_request` seed** — `workflow_run: [requested]` seeds `pending` from the base repo's token, which works for fork heads too, so the fork token problem never arises. |
| 4. `merge_group` compatibility | **OPEN — the one risk the repo cannot settle.** That `workflow_run` fires for merge-group-triggered sibling runs, and that the status lands on the ref the ruleset checks, is a stage-1 verification item. Recorded as an assumption in the workflow header, not a claim. |
| 5. Event storms | **Resolved** — per-head-SHA concurrency group with `cancel-in-progress`, plus a settle window that absorbs the burst. |
| 6. Startup-race parity | **Resolved, more fail-closed than the poll loop** — `MIN_POLLS` becomes a confirm re-fetch, and an empty sibling set never passes (the poll loop's stable-empty pass is not carried over). Both pinned by `TestSlotlessEvaluation`. |

Until the migration completes, the adaptive budget remains the operative mitigation
for the saturation false-RED, and the merge-queue/org plans reduce exposure.
