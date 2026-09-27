# Getting the ci-summary gate off the build-runner slot — design record

> Status: **DESIGN-ONLY. No code in this repo implements it yet.**
> Originally written as the sq-90cv4 follow-up (Claude Fable 5 `[FABLE-5]`); **revised
> 2026-08-31 under bead sq-6vshe.19** `[OPUS-5]`, which re-scoped the work with a hard
> constraint that invalidates the original recommendation. §2 records the corrections.
> Hardened 2026-09-03 `[GPT-5.6-SOL]` after round-two review to make transport-row
> ownership, exclusion order, and shadow/resident independence explicit.
> The operative mitigation today remains the sq-90cv4 *adaptive saturation budget* in
> `scripts/ci_summary_gate.py`.

## 1. Problem

`gate` — the single REQUIRED context in branch-protection ruleset 17688455, produced by
the `gate` job of `.github/workflows/ci-summary.yml` — is a *waiter*: a GitHub-hosted job
that polls sibling check-runs until they are all terminal. While polling it occupies one
job slot in the account-wide hosted-runner concurrency pool.

Verified against the checkout (`Config` in `scripts/ci_summary_gate.py`):

| Knob | Value | Meaning |
| --- | --- | --- |
| `interval` / `sat_interval` | 20 s / 40 s | poll cadence, base / extension phase |
| `min_polls` | 3 | startup-race floor — no verdict before 3 polls |
| `settle_polls` | 2 | all-terminal must hold this many consecutive polls |
| `base_polls` | 110 | base budget (110 × 20 s ≈ 37 min) |
| `max_total_polls` | 155 | absolute cap (+45 × 40 s ≈ 30 min) |
| `progress_window` | 15 | polls over which a rising completed-count means progress |
| `unsat_confirm_polls` | 3 | polls the unsatisfiable-hold state must persist |
| `max_consec_fetch_failures` | 5 | consecutive fetch failures before RED |

with `timeout-minutes: 80` on the `gate` job in
`.github/workflows/ci-summary.yml`.

Per `research/ci-mergequeue-speedup-2026-07.md` §5 the gate holds a slot ~15–23 min per
merge-group entry, across the concurrent queue entries plus every open PR's own gate plus
the `push: branches: [main]` run — several slots doing no compute during exactly the
bursts when the pool is tightest. (Removing the *push* waiter is sq-6vshe.14, a separate
bead; it is not addressed here.)

The sq-90cv4 adaptive budget fixed the *false verdict* under saturation. It does not fix
the *slot occupancy* — a saturated pool now holds gate slots longer, the correct trade for
verdict honesty, but it leaves throughput on the table.

## 2. What this revision changes

The 2026-07 version of this record recommended re-publishing the verdict as a **commit
status** and **migrating branch protection** to that context, listing the migration as
design risk #1. Three corrections, each verified against the checkout:

1. **The context name stays fixed, but its integration must migrate.** sq-6vshe.19
   constrains the required check name to remain exactly `gate`, which forecloses the old
   commit-status plan. An API-created *check-run* can retain that name, but §4 shows why it
   must come from a dedicated publisher App rather than GitHub Actions App 15368. The
   integration-id change is a drained, transactional ruleset migration; if sq-6vshe.19 is
   interpreted as pinning App 15368 as well as the name, this design is infeasible and the
   recommendation is to defer cutover.
2. **Pure event-driven re-dispatch is unsound on its own.** The original §2 proposed
   `workflow_run`-triggered re-evaluation as a complete replacement. It cannot satisfy
   sq-6vshe.19's own invariant *"a genuine hang still REDs"*, because a hang produces no
   completion events to trigger on (§3). A timer lane is load-bearing, not optional.
3. **The original §3.5 debounce advice was backwards.** It suggested a per-head-SHA
   concurrency group with `cancel-in-progress: true`. Applied to an evaluator, that
   cancels evaluations — including, potentially, the one that would observe the final
   all-terminal set — and hangs the gate (§6).

The original §4 claim still holds and is the reason this is tractable at all: the verdict
brain (`render_verdict`, `forgive_superseded`, `failfast_failures`, `is_advisory`, …) is
already extracted and unit-tested, so only the *transport* changes.

## 3. Finding A — a hang emits no events (the load-bearing flaw)

Every false-RED protection in `run_gate` is a **counter over polls at a guaranteed
cadence**: `min_polls`, `settle_polls`, `progress_window`, `unsat_confirm_polls`,
`max_consec_fetch_failures`, `base_polls`, `max_total_polls`, plus the fail-fast grace
re-poll (the branch in `run_gate` that immediately re-fetches and must re-observe the
identical failure set). These are meaningful only because the resident driver guarantees
one observation every 20 s.

An evaluator invoked on `workflow_run: completed` fires at the *sibling completion rate*,
which is neither periodic nor guaranteed:

- **Bursty.** A matrix shard set finishing together yields many invocations within
  seconds. `settle_polls = 2` would then be satisfied by two observations milliseconds
  apart — collapsing the post-terminal settle window that sq-ipkku exists to enforce.
- **Zero during a hang.** `progress_window = 15` polls (≈ 5 min of wall-clock) becomes 15
  *completions*, which by definition never arrive when work is stuck, so the saturation /
  hang discrimination never evaluates — and the `max_total_polls` RED never fires at all,
  because with no events there is no invocation to fire it.

Precision on severity: a `gate` check-run that never concludes still **blocks** the merge
(branch protection treats a pending required check as blocking), so this is a **liveness**
defect, not a safety hole. But the consequences are real and are exactly what sq-6vshe.19
names: the PR never learns it is red; `fast-fix-ring.yml` (triggered by `workflow_run` on
ci-summary's *completion*) never rings; the merge-queue entry sits until the queue's own
timeout.

**Therefore:** the counters must be re-expressed as **wall-clock deadlines** anchored to a
persisted start timestamp, *and* a mechanism must guarantee a minimum invocation rate.
Only a scheduled lane provides the latter. This repo already operates sub-hourly cron
sweepers, so the primitive is proven here rather than hypothetical — though note the
finest cadence in use is 5 minutes, on exactly one lane:

| Lane | `cron` | Cadence |
| --- | --- | --- |
| `merge-group-watchdog.yml` | `2,7,12,17,…,57 * * * *` | 5 min |
| `auto-arm.yml` | `4,14,24,34,44,54 * * * *` | 10 min |
| `promote-on-approval.yml` | `*/10 * * * *` | 10 min |
| `batch-merge.yml` | `7,22,37,52 * * * *` | 15 min |
| `ci-latency-alarm.yml` | `26 * * * *` | hourly |

## 4. Finding B — context identity requires a dedicated publisher App

The Checks REST API can create a check-run named `gate` on an arbitrary head SHA, but
write access is restricted to GitHub App installation credentials. A user token or classic
PAT is not an equivalent fallback. The official API contract is the authority here:
<https://docs.github.com/en/rest/checks/runs>. GitHub documents that Actions' repository
`GITHUB_TOKEN` is itself a GitHub App installation token:
<https://docs.github.com/en/actions/concepts/security/github_token>.

That convenient identity is exactly the problem. Ruleset 17688455 pins `gate` to GitHub
Actions App 15368, and every ordinary Actions job check uses that App identity regardless
of its token's `checks: write` permission. Branch protection does not inspect
`external_id`. A PR-controlled job can therefore collide with the required name before a
default-branch evaluator runs. No post-hoc envelope validation repairs that race.

The safe design uses a dedicated publisher App and ultimately pins the required context
to that integration. Phase 4 must prove its create/update identity and repository-scoped
installation behaviour on same-repository, fork, and merge-group heads. Cutover is
forbidden if that distinct identity is absent, if its credential is reachable from target
code, or if administrators will not approve the ruleset integration migration.

Two more consequences must be designed for, not assumed away:

- **The current job remains during migration.** It continues to satisfy the App-15368
  rule while the dedicated publisher is shadowed, then coexists under the same display
  name while the ruleset distinguishes the two App integrations. Strict transport
  classification in §6.2 keeps the verdicts independent.
- **The integration switch is an administrative transaction, not a bypass commit.** The
  queue is drained, every eligible head is pre-seeded with the dedicated-App `gate`, and a
  full ruleset projection changes only the required integration. The resident job is
  removed later by an ordinary protected PR. `TestRequiredCheckAnchor` is re-pointed at the
  trusted API publisher, never weakened or deleted.
- **Draft-tier integrity gets structurally weaker.** Today the invariant *"a draft-tier
  result can never satisfy branch protection"* is enforced by a YAML name expression
  (`gate${{ … ', draft-tier' … }}` on the `gate` job) — it holds even if the script is
  wrong. Under an API-created check-run the tiering moves into evaluator *code*, a weaker
  guarantee. This must be re-pinned by a unit test asserting the evaluator never names a
  check-run `gate` on a draft-tier evaluation.

## 5. Finding C — the tests pin the refactor to "extract, don't rewrite"

Most tests in `scripts/tests/test_ci_summary_gate.py` drive the full loop through its
`run(cfg, polls, …)` helper, which calls `run_gate` with a scripted list-per-poll
`fetch_runs`, a constant `fetch_queue_depth`, and a no-op `sleep_fn`. A smaller set is
cadence-sensitive: it asserts exact `fetch.state["calls"]` counts, and one test asserts
on literal `"attempt 2"` / `"attempt 3"` log text.

sq-6vshe.19 requires those tests be **extended, not weakened**, and the verdict semantics
**bit-identical**. That rules out rewriting the loop. The only shape that satisfies it:

> Extract a pure `step(state, observation, cfg) -> (state, decision)` from the body of
> `run_gate`, and keep `run_gate` as a thin `while` loop over `step`. All 186 existing
> tests then pass **unchanged**, because the resident driver's behaviour is unchanged.
> The event-driven evaluator becomes a **second driver** over the same `step`.

This is a constraint derived from the invariant, not a stylistic preference.

## 6. Finding D — the state store and its write race

State that must survive between invocations is small (well under 1 KB of JSON): the settle
counter, the completed-count history, the fail-fast suspect set, the unsatisfiable-hold
counter, consecutive fetch failures, `extension_started`, and the new start timestamp.

**Where:** the evaluator-owned check-run on the head SHA. Its `external_id` is a short,
versioned identity (`sparq-gate:v1:<phase>:<repository-id>:<head-sha>`); its `output.text`
carries a versioned machine envelope containing the same phase, repository id, head SHA,
schema version, and JSON state. Before loading state, the evaluator validates all of those
fields, the check-run's exact phase name (`gate-shadow` before cutover, `gate` after it),
and the dedicated publisher App id. The codec rejects a mismatched identity or unsupported
schema version, preserves the human verdict summary separately, and tolerates unknown
fields within a supported version for forward compatibility.

### 6.1 The publisher check is transport state, never verdict input

The persistent API-created publisher is not a sibling result. It therefore needs a
separate exclusion from the current `is_self` predicate, which is intentionally scoped to
the ephemeral Actions run identified by `SELF_RUN_ID`.

The composite identity below is a classifier, not a credential. `external_id` and the
state envelope are caller-controlled and updateable. More importantly, GitHub Actions
automatically creates job check-runs as App 15368 even when the job has no `checks: write`
token permission. An untrusted PR/merge-ref workflow can therefore emit a job named
`gate` under that App before an evaluator has a chance to reject its missing envelope.
Branch protection matches the context name and integration; it does not authenticate the
envelope. A scanner or inventory cannot close that registration race.

Cutover consequently requires a **dedicated publisher GitHub App** whose installation
credential is available only to the protected default-branch broker/evaluator. The
ruleset must pin `gate` to that App's integration. If that integration migration is not
approved or cannot be made, the slotless transport is rejected and the trusted resident
gate remains required. App id identifies the GitHub App, not a particular installation;
repository-scoped lookup, validated target identity, and exclusive control of that
repository installation's credential provide the rest of the provenance boundary.

Before shadow activation, a machine gate pins the repository's default token posture and
inventories every reserved-name producer: automatic Actions job checks (including
expression-, matrix-, and reusable-workflow-derived names), API check writers, and every
job or environment able to obtain the dedicated App credential. Only reviewed
default-branch code may receive that credential, and it may not execute or interpolate
PR/merge-ref content. Any unlisted producer, secret/environment grant, dynamic-name gap,
or privilege/trigger/checkout drift fails the gate.

Each evaluation first fetches the raw check-runs for the target SHA with `filter=all`,
paginates every page, and validates the publisher candidates **before** converting any
rows into the observation consumed by `step()`. It fails closed if pagination is
incomplete, the API reports more rows than were retrieved, or the endpoint's maximum
enumerable result set is reached. The evaluator validates raw check ids and each check
suite's repository, head SHA, and App identity rather than trusting a summarized or
`filter=latest` view:

1. enumerate every check-run whose name collides with the phase's publisher name, after
   authenticating any explicitly permitted migration transport under §6.2;
2. accept exactly one existing publisher only when its dedicated App id, `external_id`, repository
   id, head SHA, phase name, and state-envelope schema all match the trusted values;
3. create the publisher only when no colliding row exists; and
4. fail closed on more than one candidate or on any otherwise-unclassified same-name row
   with a mismatched App, external id, repository/head identity, or schema. A display-name
   match alone never grants exclusion or ownership.

"Fail closed" is a durable check transition, not merely an evaluator exception. On a
collision, corrupt envelope, or identity mismatch, the handler first changes every
dedicated-App row with the reserved phase name — including a previously trusted green
publisher — to terminal failure, then GET-verifies each transition before returning an
error. A foreign-App collision that was not authenticated as migration transport cannot be
mutated, remains visible as evidence, and still causes the trusted publisher to be
neutralized. Failure to neutralize and verify every
publisher-capable row pauses admission and raises an operator incident; it may never leave
a known prior green as the authoritative outcome.

Immediately before publishing a terminal success, the evaluator repeats the complete raw
enumeration and identity validation under the per-head lock. After validation, the one
accepted publisher row is removed in every state — queued,
`in_progress`, completed, and reopened — and every other row remains. This filtering is
the first transformation of the raw check set: it happens before workflow-run resolution,
supersession/forgiveness, fail-fast selection, terminal counts, and `step()` /
`render_verdict`. Consequently the transport row can neither hold its own verdict open nor
lend a stale success to a replacement generation, while a forged or ambiguous row cannot
be forgiven as superseded evidence.

The adversarial unit suite must drive at least two evaluator invocations over one head:
exclude the valid publisher while it is `in_progress`, complete it, request replacement
sibling work, reopen it, and exclude it again while the real pending sibling keeps the
decision non-terminal. Companion cases cover a completed publisher, duplicates split
across pagination boundaries, API truncation, a `filter=latest`-hidden older collision,
same-name/wrong-App rows, and wrong repository, SHA, check-suite identity, external-id, or
schema markers. Prior-green variants prove that wrong-App, duplicate-valid, and
same-App/bad-marker collisions first make every publisher-capable result non-successful.
Only the unique fully validated publisher is ever removed; every mismatch reaches a
durable fail-closed transport decision before the verdict pipeline runs.

### 6.2 Shadow and resident transports must not observe each other

Activating a bare `gate-shadow` beside the resident gate would create a dependency cycle:
the resident waiter would treat the undeclared shadow row as gating, while the shadow
evaluator would wait on the resident `gate`. It would also make a parity comparison
tautological if the shadow simply copied the resident verdict. The ordinary advisory
registry cannot solve this: it binds names to YAML workflow/job identities, whereas the
publisher is an API check-run, and a name-only exception would let an untrusted collision
disappear.

The transport partitioner from §6.1 is therefore shared by both drivers and lands before
shadow activation:

- the resident driver removes only the unique, fully validated dedicated-App publisher
  from its raw check set under either the shadow name `gate-shadow` or the migration name
  `gate`; mixed names, duplicates, or invalid identity fail it closed;
- the shadow driver removes that same publisher and every authenticated legacy gate-job
  attempt, then independently evaluates all real sibling lanes; and
- a separate evidence collector compares the two terminal verdicts and their normalized
  observation digests after both drivers finish. Neither verdict is an input to the other.

Legacy transport rows receive equally strict provenance checks. A `details_url` is only a
locator for the Actions job API, never proof by itself. The evaluator resolves the job and
run and requires the job's `check_run_url` to name the exact raw check id, the job/run ids
to agree, the check and run to target the same repository and head SHA, the App id to match
the trusted GitHub Actions integration, the job name to match the target's expected
full/draft legacy gate name, and the run to use the pinned `ci-summary` workflow id/path on
an allowed event. All validated legacy attempts are transport and are removed before the
generic newest-run/supersession logic. A bare name match, malformed URL, lookup failure,
wrong workflow/event/head/repository/App, or mixed valid-and-invalid collision fails the
shadow evaluation closed.

That legacy classifier runs **before publisher collision handling** on migration heads and
continues after cutover while old Actions attempts can still exist. A legacy resident
Actions job named `gate` is transport, not a dedicated-App publisher; only the complete
job/run/workflow proof above permits its exclusion. The same rule prevents an arbitrary
App-15368 job with a reserved name from being mistaken for trusted legacy transport.

The phase transition reuses the existing dedicated-App publisher check-run id rather than
leaving an old shadow row behind. With admission paused, one PATCH changes `gate-shadow`
to `gate`, changes the phase in `external_id` and the state envelope, and sets the row to
`in_progress`. A GET plus exhaustive scan of **both** reserved phase names must then prove
there is no retiring shadow, exactly one new publisher, and only strictly authenticated
legacy `gate` attempts. A duplicate or mixed old/new state is neutralized per §6.1 and
blocks migration. Every existing non-draft PR head is migrated and evaluated before the
ruleset integration changes; active merge groups are drained rather than migrated.

Tests pin the no-cycle property in both directions and prove that shadow success is
independent: an in-progress publisher cannot hold the resident waiter open; an
in-progress, failed, or stale legacy gate cannot hold or force the shadow verdict; real
sibling pending/failure rows still do. Before the phase rename, companion tests prove a
dedicated-App `gate` that is pending, failed, or stale cannot hold or force the resident
verdict. Shadow publication and migration-name publication are each forbidden until this
resident filter is merged and live on the default branch.

The authenticated resident `ci-summary / gate` remains the sole native target-SHA gating
workflow through the integration cutover. Its driver first ships with an **observe-only,
dormant** bootstrap guard while the broker and manifests do not yet exist. After the broker
path has been deployed and verified on de-armed canaries, the guard becomes fail-closed:
success requires the unique dedicated-App publisher, a valid current-generation sealed
manifest, and every pending sibling row named by that manifest to exist. The publisher
itself is excluded as transport; the manifest's sibling rows remain verdict input. A
missing publisher, invalid/incomplete manifest, or missing expected row holds pending and
then REDs at its wall-clock bootstrap deadline — it can never take the current stable-empty
success path.

Before activation, the broker seeds every eligible current head and drives a fresh resident
attempt to non-terminal as specified in §6.3; activation is refused while any head still
depends on historical green. Tests delay the broker beyond `min_polls` for both PR and
merge-group heads and prove the active guard cannot turn green. This guard, its activation
state, and the sole-native-workflow exception are pinned in the launch inventory.

### 6.3 Generation sealing prevents stale terminal success

`workflow_run: requested` and `workflow_run: in_progress` are liveness wake-ups, not an
ordering primitive. Delivery and evaluator startup can be delayed, especially under the
runner saturation this design is meant to relieve. They therefore cannot by themselves
guarantee that a terminal-success publisher is reopened before a sibling rerun,
replacement, or late reporter becomes pending.

Every automated mechanism that can create or rerun a target-SHA gating row uses one
trusted default-branch generation broker. Native target-code workflows no longer publish
their own target-SHA gating rows; the broker creates pending sibling rows and launches
unprivileged workers, and trusted reporters update only the broker-created rows. Under the
same per-head lock, the broker:

1. validates current target membership and the unique publisher using the complete raw
   enumeration contract in §6.1;
2. updates the publisher to `in_progress`, clears its terminal conclusion, records a
   monotonically increasing generation nonce, and marks the manifest incomplete;
3. GETs the publisher again and verifies its id, repository/head/App identity, nonce, and
   non-terminal state;
4. before the integration cutover, requests a fresh authenticated resident attempt and
   GET-verifies that the App-15368 `gate` is non-terminal; if that cannot be proved, the
   target must already be de-armed or admission must be globally paused;
5. creates every pending sibling row, seals the complete generation manifest, and
   GET-verifies it; and only then
6. revalidates target/admission state and dispatches or reruns the unprivileged work with
   authenticated generation/run correlation.

Step 4 is the pre-cutover stale-green interlock: reopening a non-required shadow publisher
alone is not credited with revoking the resident required result. The resident sees the
new incomplete manifest and its active bootstrap guard holds while the broker builds the
generation. No automated same-SHA label or state mutation that changes the gating manifest
may occur outside this transaction. If a target merges or leaves admission before the
final revalidation, the broker aborts without launching work.

For a genuinely new head with no publisher, the missing dedicated-App required context is
already fail-closed; the broker creates it pending before any target row. If a SHA has been
seen before — including reopen, re-arm, or reuse by another PR — the broker treats it as a
same-SHA generation and reopens the existing publisher before launching work. Initial and
replacement generations therefore use one ordering rule rather than assuming a SHA is
novel from an event type.

The evaluator may return success only after a final §6.1 re-read proves that every gating
row belongs to the sealed manifest and is terminal. Any unknown, unsealed, future, or
mixed-generation row fails closed. Native `pull_request`, `merge_group`, label, readiness,
and reporter launch paths are machine-inventoried and may only ring a non-gating doorbell;
every target-row launch goes through the broker. Inventory alone is not credited with
ordering — the create/reopen verification and complete pending manifest precede dispatch.

GitHub write-permission holders can still invoke Actions reruns or gating label/state
changes directly, so they are an explicit trusted-operator boundary. The runbook forbids
those direct UI/API mutations while a PR is armed or a merge-group head is active: the
operator first removes it from admission or de-arms it, then asks the broker for a new
generation. If the deployment threat model requires technical enforcement against
repository write administrators, this platform route cannot provide it and cutover is
rejected.

An adversarial integration test pauses the broker between the verified publisher reopen
and replacement dispatch for both a PR and a merge group. During shadow trials the targets
remain de-armed and the test proves the ordering directly. A pre-cutover prior-green case
proves the fresh resident attempt is non-terminal before any replacement row is created;
after the dedicated integration becomes required, a canary proves `gate` is already
pending throughout that window and the target cannot merge until the new generation
finishes. Companion tests reject direct/unsealed reruns, late reporters, mixed generations,
and a publisher that changes between PATCH and verification. Latency samples are useful
operational evidence but do not substitute for this ordering proof.

**The race:** N siblings completing at once means N concurrent evaluations
read-modify-writing the same state. The original §3.5 advised `cancel-in-progress: true`
to debounce. That is wrong in a way that matters:

- Cancelling an in-flight evaluation mid-write can leave torn state.
- Worse, if the cancelled invocation is the one carrying the *final* sibling's completion,
  no further event ever arrives and the gate hangs forever.

**Correct:** every event and sweep dispatch enters the **same** per-head-SHA concurrency
group with `cancel-in-progress: false`. The design uses that group only to prevent two
evaluators from mutating one publisher concurrently; it does **not** depend on the number,
ordering, replacement, or retention of pending runs. Each invocation performs a **full
re-read of the world** (`fetch_runs()` re-fetches every check-run from scratch), never
applies the event as an incremental delta, and the sweep guarantees a later evaluation.

Application-level coalescing is an idempotent fast path in the persisted state: record a
digest of the validated, transport-filtered observation plus the next wall-clock deadline.
A serialized invocation that sees the same digest and no deadline due exits without
advancing counters or writing the publisher. If the observation changed, or a deadline is
due, it evaluates and stores the new digest atomically with the state update. Extra queued
invocations are therefore harmless fast no-ops rather than a correctness assumption about
platform queueing. The concurrency primitive's role is documented by GitHub's concurrency
contract:
<https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency>.

Creation must also be idempotent. Under the concurrency lock the evaluator applies the
identity and collision rules in §6.1, creates one publisher only when there is no
collision, and never silently picks a winner from duplicate required contexts. Publisher
reopen and sibling launch follow the generation-sealing protocol in §6.3; observing an
asynchronous event is never used to claim that stale success was revoked in time. The
Checks update contract permits the App-owned run's status to be updated, and the broker's
PATCH/GET ordering is covered by the adversarial tests in §6.3.

## 7. Recommended architecture

The design separates unprivileged target work, non-verdict doorbells, the privileged
generation broker/evaluator, and the sweep. No workflow that executes a PR or merge ref
receives the dedicated App credential or dispatch privilege.

### 7.1 Non-verdict doorbells and broker-launched workers

A small PR doorbell runs on `pull_request_target`, so GitHub takes its definition from the
default branch. It never checks out a ref, restores a cache, downloads an artifact, or
executes a repository script. Its token has narrowly scoped dispatch permission and no
contents/checks/PR write; its only operation invokes the generation broker at the exact
default-branch ref, passing event kind and PR head SHA as untrusted data through environment
variables. Same-SHA label, readiness, reopen, and manual automation events ring this
doorbell rather than launching target work themselves.

For merge groups, the existing default-branch doorbell invokes the same broker on
enqueue/dequeue. It is the only native `merge_group` target trigger. There is no
target-ref-defined writer or dispatcher whose definition could come from the combined ref.

Neither doorbell can publish or update a verdict. If one is absent or fails, the required
context remains absent and the scheduled sweep below is the backstop.

The broker validates the target, acquires the per-head lock, registers or reopens the
publisher, and creates the complete set of pending sibling check-runs for the new
generation before it starts compute. It then invokes worker jobs whose workflow definitions
come from the reviewed default branch. A worker checks out the target SHA with persisted
credentials disabled and receives no dedicated-App credential, repository write token, or
untrusted cache restore. Its result is data returned to a separate trusted reporter, which
matches broker-run id, worker job id, lane, repository, head SHA, and generation nonce
against the sealed manifest before updating that one sibling row.

Except for the authenticated resident `ci-summary / gate` migration anchor in §6.2, all
gating workers lose native `pull_request`, `merge_group`, `labeled`, `unlabeled`,
`ready_for_review`, and target-triggered reporter entry points. A repository test enumerates
those event forms, reusable workflows, workflow dispatches, rerun helpers, and API writers
in both directions: only non-gating doorbells and that exact guarded resident may start
natively, and only the broker may create new target-SHA gating rows. This conversion is a
prerequisite, not an optimization.

### 7.2 Default-branch evaluator

`ci-gate-eval.yml` has **no** `pull_request` or `merge_group` trigger. Its privileged job
runs only on:

- `workflow_run` with `types: [requested, in_progress, completed]` and an explicit
  `workflows:` list;
- `workflow_dispatch` with a target SHA, dispatched at the exact default-branch ref by
  the sweeper or a trusted reporter.

GitHub can deliver `workflow_run` without a `workflows:` filter, but this privileged design
deliberately requires an explicit list: it constrains the wake-up surface, makes the list
auditable, and lets repository tests fail on inventory drift. No wildcard form is used.
`in_progress` is included as a liveness wake-up because GitHub does not emit the
`requested` activity for a re-run. It is not the stale-green interlock; the synchronous
broker ordering in §6.3 provides that guarantee.
The event also runs the evaluator definition from the default branch and can receive a
write token even when the upstream workflow was unprivileged. Those are documented
platform properties, not local assumptions:
<https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#workflow_run>.

The list is generated from a committed wake-up/launch inventory and pinned in both
directions by a drift test:

1. every workflow with a PR, merge-group, label, readiness, or manual/rerun trigger is a
   reviewed non-gating doorbell, the exact bootstrap-guarded resident gate, or is rejected;
   no other native workflow can publish a target gating row;
2. every broker-launched worker and expected sibling name exists exactly once in the
   sealed manifest, and every completion wake-up name appears byte-for-byte in
   `ci-gate-eval.yml`;
3. every default-branch reporter that creates check-runs on another head SHA routes its
   launch through the generation broker, which opens the publisher before the first write,
   and explicitly dispatches the evaluator after its last write;
4. `pr-area-label` remains an unconditional manifest row for every PR and merge-group
   target; the two default-branch doorbells' no-checkout, no-check-write, dispatch-only
   posture and the resident gate's sole-native exception/bootstrap guard are pinned.

An unknown workflow therefore fails the routing self-test; it cannot silently disappear
from the wake-up set. The inventory controls **when to wake**, not what the verdict sees:
each invocation still re-reads all workflow runs and check-runs on the target SHA, applies
the exact transport-identity validation and publisher exclusion in §6.1, and presents all
remaining rows to the unchanged verdict semantics.

The evaluator checks out and executes only its triggering default-branch SHA, with
credentials persistence disabled. A dispatched run refuses to evaluate unless its
workflow ref is the default branch and its execution SHA belongs to that branch's reviewed
history. It never checks out the target SHA, restores a cache from it, downloads its
artifacts, or executes text from the event payload. The target SHA is data: validate its
syntax, repository, current PR/merge-group membership, and target kind before any write.
Event fields reach scripts through environment variables or JSON files, never shell
interpolation.

The workflow token keeps only the current read set and the narrowly bounded
`actions: write` operations used by the broker/re-dispatch path. A short-lived installation
token from the dedicated App holds `checks: write` only for the publisher repository. The
job enters the per-SHA concurrency group from §6, validates/loads state, removes exactly
the trusted publisher row per §6.1, performs one `step()`, repeats the complete identity
and generation read before success, and updates either `gate-shadow` or (only after
cutover) `gate`.

### 7.3 Default-branch sweep

`ci-gate-sweep.yml` runs at GitHub's minimum supported schedule interval and on manual
dispatch. It holds read permissions plus only the permission needed to dispatch the
evaluator; it does **not** hold `checks: write`. It enumerates current non-draft PR heads,
active merge-group heads, and non-terminal publishers that pass the same repository/head/
App/external-id/schema identity checks from §6.1, then dispatches `ci-gate-eval.yml` at the
exact default-branch ref once per SHA. A same-name row that fails those checks is an alert,
not a sweep target. The dispatched runs join the same concurrency groups as event wake-ups.

`pr-area-label` is the one unconditional expected source workflow per target. The
evaluator records the target discovery time and refuses a stable-empty success until that
run is observed successful and the existing startup/settle semantics are satisfied. If
it never registers or never becomes terminal by the corresponding wall-clock deadline,
the sweep drives `gate-shadow`/`gate` to a terminal failure. A total Actions outage may
delay that diagnostic, but the missing required context remains fail-closed throughout.

## 8. Honest payoff analysis — peak concurrency, not billable minutes

This is where the bead's estimate needs qualifying.

- **Peak concurrent slot occupancy** drops from N resident waiters to a few ephemeral
  jobs. This is the metric that actually produced the 2026-07-02 congestion collapse, and
  the win here is real.
- **Total slot-seconds may barely improve, or may worsen.** Each evaluation pays fixed
  startup overhead — runner acquisition, the sparse `actions/checkout`, Python start —
  that the resident waiter pays exactly once. `docs/branch-protection.md` enumerates on
  the order of dozens of aggregated lanes per head, so the sum of those overheads is
  plausibly comparable to the resident wait it replaces.

So sq-6vshe.19's "frees 3–6 runner slots" is a **peak-concurrency** claim and must not be
restated as a billable-minutes saving. **This is also the decision point for the bead's
option (c) (reject with measurement):** if measurement shows the pool is bound by total
minutes rather than peak concurrency, this re-architecture is not worth its risk and the
adaptive saturation budget stays the operative mitigation.

## 9. Phased plan (each implementation phase is a future bead)

1. **Phase 0 — measure, then go/no-go.** Preserve the live Actions API snapshot linked in
   this PR's review discussion as structured evidence, then add a repeatable collector for
   waiter occupancy and sibling progress. Confirm peak concurrency, rather than only total
   minutes, is binding. A negative result chooses option (c) and stops here.
2. **Phase 1 — pure refactor, zero behaviour change.** Extract `step()`; `run_gate` becomes
   a loop over it (§5). All existing tests pass unchanged; add tests for `step()` purity.
   Independently mergeable, no risk to the required check.
3. **Phase 2 — wall-clock state and transport partitioning.** Re-express poll counters as
   deadlines anchored to a persisted start timestamp, keeping the resident driver's fixed
   cadence equivalent;
   add the fail-closed state codec, observation digest, transport-owned publisher filter,
   legacy-transport validator, exhaustive raw-check pagination, reserved-name/credential
   inventory gate, durable collision handling, the resident sealed-manifest bootstrap guard
   in dormant observe-only mode, and multi-invocation adversarial tests (§6). Provisioning
   a dedicated publisher App and its protected environment is a hard prerequisite. Preserve
   the resident driver's existing `SELF_RUN_ID` ordering and tests unchanged outside the
   dormant guard.
4. **Phase 3 — broker the launch topology.** Add the generated wake-up/launch inventory,
   its both-direction drift test, the default-branch PR and merge-group doorbells, sealed
   sibling manifests, unprivileged worker execution, and trusted result reporters
   (§6.3–§7.2), initially on de-armed canaries while old sibling triggers remain. Once that
   path is verified, seed every eligible head, request and verify fresh non-terminal
   resident attempts, and activate the bootstrap guard. Only then remove every native
   target trigger from gating workers, including same-SHA label/readiness and rerun helpers,
   except the exact authenticated resident gate. That guarded resident remains
   authoritative while this large migration is exercised; failure to achieve complete
   broker ownership chooses defer/reject rather than partial cutover.
5. **Phase 4 — shadow transport.** First merge and verify the resident driver's strict
   dedicated-App transport exclusion under both `gate-shadow` and migration `gate` names;
   only then activate the evaluator and sweeper. The
   dedicated App publishes the non-required `gate-shadow` context, excludes authenticated
   legacy gate transports, and exercises PR, fork, initial run, broker-sealed rerun,
   no-leg, cancellation, genuine-hang, late reporter, merge-group, publisher reopen,
   wrong-App/name collision, pagination/truncation, and duplicate-publisher cases.
6. **Phase 5 — evidence gate.** Compare every terminal shadow verdict with the resident
   verdict and measure wake/terminal latency. Audit the complete reserved-name and
   credential inventory, prove generation sealing, and prove the two transports never
   observe one another. Then, while the resident Actions `gate` remains required, migrate
   each non-draft PR head's dedicated-App publisher from `gate-shadow` to `gate` using the
   single-row protocol in §6.2 and repeat parity collection by check id/App. Any unexplained
   mismatch, duplicate, missing target, stale-success window, writer drift, dependency
   cycle, or permission failure blocks cutover.
7. **Phase 6 — controlled integration cutover.** Drain the merge queue and pause admission.
   Prove every eligible PR head has exactly one current successful dedicated-App `gate`
   and one authenticated resident result. With explicit administrator authorisation,
   apply a full ruleset projection that changes only the required `gate` integration from
   GitHub Actions App 15368 to the dedicated publisher App; the required context name and
   every other rule remain unchanged. No branch-protection bypass or code commit is needed.
   Evaluate a canary PR and fresh merge-group head before reopening admission. If a
   dedicated integration cannot be selected, stop: the App-15368 design is not safe to
   cut over.
8. **Phase 7 — cleanup.** Remove the resident polling transport, keep the rollback commit
   prepared, and update `ci-summary.yml`'s doctrine header and
   `docs/branch-protection.md`.

**Rollback.** Before Phase 6, disabling the shadow has no merge effect. During the Phase-6
canary window the resident Actions gate stays live. A pre-cleanup rollback pauses
admission, drains active merge groups, and first proves a fresh authenticated resident
result for the current sealed generation of every eligible head; only then may the
captured full ruleset projection restore App 15368. Historical green rows are never valid
rollback evidence.

After Phase 7, rollback is a two-stage recovery while the dedicated integration remains
required: land the reviewed resident implementation again through the normal protected
path, then refresh every eligible PR head (or use an equally reviewed trusted trigger) so
the restored native workflow emits a new App-15368 `gate`. With admission still paused,
authenticate and verify those current resident results and drain all merge groups before
atomically restoring the old integration projection. A missing or stale resident result
blocks the switch. Dedicated publication is disabled only after the restored rule and a
fresh canary are verified.

## 10. Decisions and live questions

Resolved by this revision:

- `workflow_run` uses a complete explicit workflow list; no wildcard is assumed.
- PR/merge-ref execution is unprivileged and separate from the default-branch evaluator.
- every wake-up is idempotent and keyed by repository + head SHA;
- the unique fully validated publisher is excluded before any verdict transformation,
  while every identity mismatch or duplicate fails closed;
- App/external-id matching is not treated as authorization: a dedicated publisher App and
  a machine-pinned inventory of every reserved-name and credential path are part of the
  trust boundary;
- resident and shadow transports are mutually excluded and compared out of band;
- replacement work is generation-sealed only after the required publisher has been
  reopened and GET-verified; before integration cutover a fresh resident attempt is also
  verified non-terminal, and asynchronous events provide liveness, not ordering;
- shadow publication precedes any required-context change;
- initial registration and a missing seed have explicit fail-closed behaviour.

Phase 4 must answer with live evidence, not maintainer guesswork:

1. Does `workflow_run` deliver both requested/completed wake-ups for merge-group-triggered
   source workflows, with the merge-group head SHA?
2. Can the dedicated publisher App be selected as the required-check integration for
   same-repository, fork, and merge-group heads, and does its installation token remain
   unavailable to all target-controlled workflows?
3. Which default-branch reporters need an explicit evaluator doorbell because their own
   `workflow_run.head_sha` is the default-branch SHA rather than the head they annotate?
4. Is peak concurrency or total slot time the binding constraint after the label-router
   and merge batching changes land?
5. Does scheduled hang detection remain timely under the same saturation it is intended
   to diagnose?

## 11. Verification status

Verified by reading this checkout: the `Config` constants and `run_gate` control flow
(`scripts/ci_summary_gate.py`); the gate job name, triggers, permissions and timeout
(`.github/workflows/ci-summary.yml`); the test counts, driver helper and cadence-sensitive
assertions (`scripts/tests/test_ci_summary_gate.py`); the required-check anchor
(`scripts/tests/test_ci_select_wiring.py`); the required context and aggregated lane set
(`docs/branch-protection.md`); and the existing cron lanes' schedules.

The 2026-09-01 revision also checked the official GitHub Actions documentation for the
optional `workflow_run.workflows` filter, default-branch trust model, and concurrency
serialization, and the Checks REST documentation for App-only writes. A live Actions API
snapshot linked in this PR's discussion confirmed that resident `ci-summary` waiters and
queued build work coexist under saturation.

Still not verified: dedicated-App installation and ruleset selection, merge-group wake-up
payloads, required-check registration and generation-sealing timing, exhaustive check-run
enumeration at live scale, and shadow/live verdict equivalence. Phases 4–5 exist
specifically to turn those platform assumptions into evidence before the required
integration changes.
