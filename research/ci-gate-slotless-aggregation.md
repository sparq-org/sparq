# Getting the ci-summary gate off the build-runner slot — design record

> Status: **DESIGN-ONLY. No code in this repo implements it yet.**
> Originally written as the sq-90cv4 follow-up (Claude Fable 5 `[FABLE-5]`); **revised
> 2026-08-31 under bead sq-6vshe.19** `[OPUS-5]`, which re-scoped the work with a hard
> constraint that invalidates the original recommendation. §2 records the corrections.
> Hardened 2026-09-03 `[GPT-5.6-SOL]` after round-two review to make transport-row
> ownership, exclusion order, and shadow/resident independence explicit.
> Corrected again after independent review on 2026-09-03 `[GPT-5.6-SOL]`: Actions
> schedules are now explicitly best-effort; broker recovery is level-triggered; App
> installation permission is separated from runtime-token authority; and removal of the
> resident waiter requires a merge-independent, already-on-main recovery transport.
> Hardened once more after protocol review on 2026-09-03 `[GPT-5.6-SOL]`: every
> gating-affecting transition now advances a durable timeline epoch; worker launch uses a
> fenced outbox plus worker-side claims; broad Actions authority is isolated outside Actions;
> the liveness claim is conditional on a measured capacity envelope; and post-cleanup recovery
> requires an independent data path and a distinct enforceable publisher identity.
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
design risk #1. Eleven corrections now constrain any implementation:

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
4. **GitHub Actions cron is not a clock.** Scheduled workflows may be delayed or dropped
   during load, and they consume the same hosted-runner capacity this design is meant to
   relieve. Actions cron remains a reconciliation hint; an independently scheduled,
   isolated-capacity control-plane path is required for a bounded hang-to-RED claim (§3,
   §7.4).
5. **Doorbells are not durable generation requests.** A failed doorbell or a replaced
   pending Actions run must not strand a target. Every sweep performs idempotent,
   level-triggered broker reconciliation from authoritative target state before evaluation
   (§6.3, §7.1, §7.3).
6. **App installation eligibility is not runtime authority.** The installation carries
   the permissions GitHub requires to publish checks and be selected as an expected source;
   each runtime token is reduced to this repository and `checks: write` only (§4, §7.2).
7. **A rollback commit is not a recovery mechanism, and App 15368 is not a recovery
   provenance boundary.** The same App/name pair cannot distinguish the reviewed recovery
   producer from a target-controlled collision. Cleanup may remove the active waiter only
   after an independent recovery state/launch/report/verdict path and a distinct recovery App
   (or another source identity branch protection can enforce) are provisioned and drilled.
   The inert App-15368 transport may remain on `main` for emergency diagnostics, but admission
   stays paused and its result is never credited toward reopening (§9).
8. **A SHA is not a gating generation.** Label, readiness, queue-membership, rerun, and
   operator transitions can move A→B→A without changing the head or final snapshot. A durable
   monotonic gating epoch and immutable timeline high-watermark are part of every desired
   digest, and are advanced before or atomically with every broker-origin mutation (§6.3).
9. **A dispatch response is not a durable launch record.** GitHub can accept a dispatch just
   before the broker loses the response. A fenced dispatch outbox and worker-side claim make
   retries safe and let a later sweep repair both stranded and duplicate launches (§6.3,
   §7.1).
10. **`actions: write` is broad.** It authorizes more than dispatch: the credential can also
    rerun, cancel, enable, and disable workflows. No Actions job receives it. A separately
    authenticated launch service holds that broad repository-scoped credential behind a
    fixed-workflow, fixed-ref API and a fenced audit/outbox contract (§7.2).
11. **Bounded liveness is conditional on a supported envelope.** Phase 5 must derive and
    publish limits for target count, due backlog, pagination, API budget/backoff, lease wait,
    dispatch backlog, and isolated compute. Admission pauses outside that measured envelope;
    the design does not promise a deadline it has not proved (§7.4).

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
persisted start timestamp, *and* a mechanism independent of the affected Actions queue must
provide a measured maximum interval between evaluations. A GitHub Actions `schedule` does
not provide that guarantee: GitHub documents that scheduled runs can be delayed under load
and some queued jobs can be dropped, while this repository has also observed long gaps in
sub-hourly schedules during saturation. The Actions troubleshooting contract is the
authority here:
<https://docs.github.com/en/actions/how-tos/troubleshoot-workflows>.

The existing cron lanes below remain useful **best-effort reconciliation signals**, not
minimum-rate clocks. Their configured cadence is not an execution SLO:

| Lane | `cron` | Cadence |
| --- | --- | --- |
| `merge-group-watchdog.yml` | `2,7,12,17,…,57 * * * *` | 5 min |
| `auto-arm.yml` | `4,14,24,34,44,54 * * * *` | 10 min |
| `promote-on-approval.yml` | `*/10 * * * *` | 10 min |
| `batch-merge.yml` | `7,22,37,52 * * * *` | 15 min |
| `ci-latency-alarm.yml` | `26 * * * *` | hourly |

Before shadow activation, the implementation must provision an independently scheduled
liveness driver whose evaluation compute does not consume the account-wide hosted-runner
pool, or capacity that is separately isolated and proven available under full pool
saturation. It runs the same reviewed broker/evaluator core described in §7, pins its code
and target repository, executes no target-controlled content, and has an explicit
`LIVENESS_SLO`, fixed at no more than five minutes from a due deadline to the terminal
publisher update within the measured supported-capacity envelope in §7.4. A genuine hang must
become RED no later than its wall-clock deadline plus that SLO while the target population,
backlog, API budget/backoff, and isolated capacity remain inside the envelope; an overrun pauses
admission and makes no unsupported deadline claim.
The Actions schedule remains an opportunistic reconciliation path for ordinary missed
events, but is never credited toward this bound. If the independent path, isolated
capacity, out-of-band alarm, and saturation evidence cannot be provided, the cutover is a
no-go and the resident waiter remains authoritative.

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

There are two deliberately different permission surfaces:

- **Installation eligibility.** The App installation has `checks: write` so it can create
  and update check-runs and, where GitHub's required-check source-selection contract
  requires it, `statuses: write` so the App can appear as an expected source. This is
  verified against the live installation and ruleset API rather than inferred from the
  manifest. GitHub documents the expected-source requirement here:
  <https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets>.
- **Runtime-token authority.** The protected publisher-mutation service mints each
  dedicated-App token for the Sparq repository id alone and reduces it to `checks: write`;
  it does not carry `statuses: write` or organization/repository administration. Actions
  adapters request fenced mutations from that service and never receive the App token or
  minting credential. They likewise receive no `actions: write` token: dispatch requests go
  through the authenticated external launch service in §7.2. That service's repository-scoped
  credential is still broad at GitHub's permission boundary — it can dispatch and rerun work,
  and cancel, enable, or disable workflows — so its reviewed API exposes only the fixed
  default-branch worker launch operation and every use is fenced and audited. GitHub documents
  repository and permission narrowing for installation tokens here:
  <https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-an-installation-access-token-for-a-github-app>.

Phase 4 must prove both surfaces independently: a checks-only runtime token can create and
update the App-owned check, cannot write a commit status, and the live ruleset readback can
select exactly that App for `gate`. A permission mismatch, an over-broad runtime token, or
an unprotected minting path is a cutover blocker.

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

Evaluator state that must survive between invocations is small (well under 1 KB of JSON): the
settle counter, the completed-count history, the fail-fast suspect set, the unsatisfiable-hold
counter, consecutive fetch failures, `extension_started`, and the new start timestamp.

**Normal-mode mirror:** the evaluator-owned check-run on the head SHA. Its `external_id` is a
short, versioned identity (`sparq-gate:v1:<phase>:<repository-id>:<head-sha>`); its
`output.text` carries a versioned machine envelope containing the same phase, repository id,
head SHA, schema version, gating epoch, sealed-manifest digest, and JSON evaluator state.
Before loading state, the evaluator validates all of those fields, the check-run's exact phase
name (`gate-shadow` before cutover, `gate` after it), and the dedicated publisher App id. The
codec rejects a mismatched identity or unsupported schema version, preserves the human verdict
summary separately, and tolerates unknown fields within a supported version for forward
compatibility.

The check-run is not the only copy needed for recovery. The isolated control plane keeps an
append-only, durably replicated per-target journal of immutable timeline receipts, the
monotonic gating epoch, desired intent, sealed manifests, evaluator checkpoints, dispatch
outbox rows, worker claims, and every requested/published transition. A journal entry is
committed before its corresponding GitHub side effect; the normal publisher envelope mirrors
the committed checkpoint digest. Normal evaluation requires the mirror and journal to agree.
A missing or mismatched side is fail-closed and reconciled from authoritative GitHub state,
never silently selected as the winner. The separate recovery plane in §7.5 may retain this
journal as audit evidence, but it rebuilds its own state from authoritative sources and cannot
require either the primary journal or failed publisher to remain available.

The shared mutation lease is deliberately not stored in the check-run it protects. The
isolated control plane provides a durable per-head lease with monotonically increasing
fencing tokens; both Actions adapters and the independent driver use it. All dedicated-App
check-run writes pass through the publisher-mutation service that exclusively holds the
reduced App token. It atomically rejects a request whose fencing token is no longer current
before issuing the serialized GitHub mutation, so an expired caller cannot resume and
overwrite newer state. Merely copying a token into `output.text` would not fence the GitHub
API and is explicitly insufficient. Normal-mode verdict state is committed in the independent
journal and mirrored in the check-run envelope; the lease namespace remains separate from both,
and the mutation service may publish only a request already authorized by the broker/evaluator
state machine. Its API accepts no free-form GitHub payload: it authenticates the reviewed caller
and revalidates repository,
head SHA, check id/name/App, expected prior-state digest, desired-generation intent, fencing
token, and requested state transition. A mismatch or an evaluator request for success
without the complete sealed-generation proof is durably failed closed. The service commits
the new evaluator checkpoint and publication intent to the independent journal before the
GitHub PATCH, then records and GET-verifies the observed result. A retry with the same
transition id is idempotent; a different transition under an old fence is rejected.

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

Cutover consequently requires a **dedicated publisher GitHub App** whose token-minting
credential is available only to the protected control plane. The ruleset must pin `gate`
to that App's integration. If that integration migration is not approved or cannot be
made, the slotless transport is rejected and the trusted resident gate remains required.
App id identifies the GitHub App, not a particular installation; repository-scoped lookup,
validated target identity, reduced runtime tokens, and exclusive control of that repository
installation's minting credential provide the rest of the provenance boundary.

Before shadow activation, a machine gate pins the repository's default token posture and
inventories every reserved-name producer: automatic Actions job checks (including
expression-, matrix-, and reusable-workflow-derived names), API check writers, and every
job, service, or environment able to obtain a reduced dedicated-App token or its stronger
minting credential. Only reviewed default-branch or pinned control-plane code may receive
either, and it may not execute or interpolate PR/merge-ref content. Any unlisted producer,
secret/environment grant, dynamic-name gap, over-broad token, or
privilege/trigger/checkout drift fails the gate.

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
trusted generation broker. Native target-code workflows no longer publish their own
target-SHA gating rows; the broker creates pending sibling rows and launches unprivileged
workers, and trusted reporters update only the broker-created rows. A doorbell payload is
never the generation intent.

Current state alone is also insufficient: on one unchanged SHA, readiness or labels can move
A→B→A and leave the same final snapshot behind a now-stale green result. The independent
control-plane journal therefore maintains, for every target, a monotonically increasing
`gating_epoch` and a high-watermark vector over immutable, ordered receipts for every
transition that can change admission, the required lane set, or the validity of existing
evidence. Those transitions include readiness, gating-label, reopen/close, enqueue/dequeue,
explicit rerun, launch-inventory revision, and protected operator commands. The broker
paginates and records
all new authoritative timeline and Actions-attempt receipts on every sweep, even when their
net effect returns to the prior state. A gap, truncation, unsupported mutation channel, or
unprovable ordering durably neutralizes prior success and pauses admission.

A committed source map assigns each transition class either to the broker's write-ahead
request stream or to a live-proven GitHub event/timeline/attempt stream with a stable immutable
id, source-specific cursor, complete-pagination rule, and replay horizon. A both-direction
repository test inventories every gating-affecting trigger and writer and rejects an unmapped
class or an unused source-map entry. Phase 5 must prove replay after dropped deliveries for
every mapped GitHub source. If the platform exposes no total, replayable receipt source for a
transition that an untrusted actor can use to affect admission, that transition cannot be
supported and cutover is a no-go; polling the final snapshot is not an alternative.

Every automated gating-affecting GitHub mutation enters through the broker. Under a fresh
fence, it first atomically appends the immutable request receipt and advances `gating_epoch`,
then reopens the publisher and only then performs the requested GitHub mutation. Platform- or
user-originated state changes are treated as untrusted proposals, not as immediately effective
gating transitions: a later webhook alone is insufficient. The independent ingress must first
durably receive the immutable source receipt, and the broker must advance the epoch and reopen
the publisher before the admission controller may reflect that state in its manifest,
auto-merge, or queue decision. If GitHub would make a supported transition merge-effective
before that interlock, cutover is a no-go. A trusted operator who can bypass the admission
interlock must first de-arm the target and request the transition through the broker; if
repository policy cannot enforce or accept that administrator boundary, cutover is rejected.

The write-ahead request remains a mutation-outbox row until an authoritative readback proves
the exact effect. A crash before or after the GitHub call retries the same receipt id under a
fresh fence; the epoch never rolls back, and failure to obtain the intended readback leaves the
publisher non-successful and admission paused. This makes the epoch ordering itself
level-triggered rather than dependent on the mutating request's response.

On every invocation the broker re-reads the receipts, authoritative current PR/merge-group
state, Actions attempts, and committed launch inventory. It computes
`H(schema, repository, target identity, head SHA, gating_epoch, timeline high-watermark vector,
admission state, inventory revision, required lanes)` as the desired-generation digest. The
independent journal records that intent, its monotonic generation nonce, and
`manifest_complete: false` before any GitHub or worker-launch side effect; the publisher
envelope mirrors the committed digest. A later reconciliation can therefore reconstruct and
finish the desired generation without the triggering event or the publisher as its sole copy.

Under the shared fenced per-head mutation lease, the broker:

1. exhaustively ingests and journals every new gating-affecting receipt, validates the
   high-watermark, current target/admission state, and publisher ownership/collision set using
   §6.1, and rejects a stale fence or incomplete source enumeration;
2. compares the epoch-bound desired digest with the durable intent and sealed manifest; if
   they already match, it reconciles the durable launch outbox, worker claims, reporters, and
   evaluator checkpoint rather than starting a duplicate generation;
3. otherwise commits an incomplete intent with the current `gating_epoch` and a monotonically
   increasing generation nonce to the independent journal, then requests the mutation service
   to create or set the publisher `in_progress`, clear any terminal conclusion, and mirror that
   intent;
4. GETs the publisher again and verifies its id, repository/head/App identity, epoch, intent,
   nonce, fencing token, checkpoint digest, and non-terminal state;
5. before the integration cutover, requests a fresh authenticated resident attempt and
   GET-verifies that the App-15368 `gate` is non-terminal; if that cannot be proved, the
   target must already be de-armed or admission must be globally paused;
6. creates or reuses exactly the pending sibling rows required by the intent and, in one
   durable transaction, seals the complete generation manifest and creates one pending launch
   outbox row per lane; it then mirrors and GET-verifies the sealed manifest; and only then
7. re-ingests the timeline, revalidates the epoch, target/admission state, and manifest, and
   asks the external launch service to pump due outbox rows. No worker can claim work until all
   those checks remain current.

Step 5 is the pre-cutover stale-green interlock: reopening a non-required shadow publisher
alone is not credited with revoking the resident required result. The resident sees the
new incomplete manifest and its active bootstrap guard holds while the broker builds the
generation. No automated same-SHA mutation that can change admission, the gating manifest, or
evidence validity may occur outside the epoch transaction. If a target merges, leaves
admission, or gains an unreceipted transition before the final revalidation, the broker aborts
without launching work and the next sweep reconciles the newer epoch.

Each outbox row has a stable
`launch_key = H(repository, target kind, head SHA, gating_epoch, generation nonce, lane)` and a
state machine `pending → dispatching → claimed → running → reported → terminal`. The launch
service records an attempt under the current fence before calling the fixed default-branch
`workflow_dispatch`. GitHub does not offer an idempotency key for that call, so a successful
HTTP response is not the exactly-once boundary. Every worker first authenticates to the
control plane with its GitHub OIDC job/run identity and attempts a compare-and-swap claim on
the stable `launch_key`. The claim service verifies the repository id, exact allowlisted
default-branch `job_workflow_ref`, reviewed execution SHA, run id/attempt, environment, and
matching pending outbox attempt against the GitHub API; knowledge of a launch key or an OIDC
token from a target-defined workflow is insufficient. Exactly one current run receives a
monotonically fenced claim token; duplicate runs receive no target data and terminate without
executing the lane. Reporters must present that claim token as well as the sealed
epoch/generation/lane identity.

If the service crashes after GitHub accepts a dispatch but before the response is recorded,
the outbox remains unresolved. A later sweep correlates any observed run or worker claim and,
after the configured no-claim deadline, may retry the same `launch_key`. The worker-side claim
makes a duplicate dispatch safe, while retry makes an accepted-but-never-started dispatch
non-stranding. If a claimed worker dies, expiry creates a higher claim fence for a replacement;
the old worker's late report is rejected. Outbox completion requires a verified terminal
sibling row and journaled reporter receipt, not merely a dispatch response or run conclusion.
Dispatch attempts, claim reassignments, and terminalization are all level-triggered and
idempotent under the shared per-head fence.

For a genuinely new head with no publisher, the missing dedicated-App required context is
already fail-closed; the broker creates it pending before any target row. Until that first
publisher exists, authoritative target membership, journaled timeline receipts, and the
committed launch inventory are the durable, reconstructible source of desired intent: both
sweep paths rediscover it without a doorbell. If a SHA has been seen before — including
reopen, re-arm, or reuse by another PR — the broker requires a later `gating_epoch` and reopens
the existing publisher before launching work. A final state equal to an earlier snapshot can
never reuse its green because the epoch and high-watermark remain different. Initial and
replacement generations therefore use one ordering rule rather than assuming a SHA is novel
from an event type.

The evaluator may return success only after a final §6.1 re-read proves that every gating
row belongs to the sealed manifest and is terminal, every outbox row is terminal, and the
timeline high-watermark and `gating_epoch` still match the independent journal. Any unknown,
unsealed, future, or mixed-generation row, unclaimed launch, late timeline receipt, or journal
mismatch fails closed. Native `pull_request`, `merge_group`, label, readiness, and reporter
launch paths are machine-inventoried and may only ring a non-gating doorbell or authenticated
external ingress; every target-row launch goes through the broker. Inventory alone is not
credited with ordering — the epoch receipt, publisher reopen verification, complete pending
manifest, and durable outbox precede dispatch.

GitHub write-permission holders can still invoke Actions reruns or gating label/state
changes directly, so they are an explicit trusted-operator boundary. The runbook forbids
those direct UI/API mutations while a PR is armed or a merge-group head is active: the
operator first removes it from admission or de-arms it, then asks the broker for a new
generation. If the deployment threat model requires technical enforcement against
repository write administrators, this platform route cannot provide it and cutover is
rejected.

Adversarial integration tests pause the broker between the epoch receipt, verified publisher
reopen, manifest seal, outbox commit, dispatch call, claim, and replacement report for both a
PR and a merge group. During shadow trials the targets remain de-armed and the tests prove the
ordering directly. A pre-cutover prior-green case proves the fresh resident attempt is
non-terminal before any replacement row is created; after the dedicated integration becomes
required, a canary proves `gate` is already pending throughout that window and the target
cannot merge until the new generation finishes. A same-SHA A→B→A case drops both doorbells:
the later sweep must ingest both immutable receipts, advance the epoch twice, reject the old
reporter, and keep the prior green unusable even though the final snapshot matches. Dispatch
cases crash immediately before and after GitHub accepts the request, start duplicate workers,
expire a winning claim, and deliver both old and replacement reports; they must show no
stranded lane, at most one active claimant, and acceptance only from the newest claim fence.
Companion tests reject direct/unsealed reruns, missing timeline pages, late reporters, mixed
generations, and a publisher that changes between PATCH and verification. Latency samples are
useful operational evidence but do not substitute for these ordering proofs.

**The race:** N siblings completing at once means N concurrent evaluations
read-modify-writing the same state. The original §3.5 advised `cancel-in-progress: true`
to debounce. That is wrong in a way that matters:

- Cancelling an in-flight evaluation mid-write can leave torn state.
- Worse, if the cancelled invocation is the one carrying the *final* sibling's completion,
  no further event ever arrives and the gate hangs forever.

**Correct:** mutation-critical broker reconciliation and disposable evaluator wake-ups do
not share a replaceable pending queue:

- broker requests use `gate-gen:v1:<repository-id>:<target-kind>:<head-sha>` with
  `cancel-in-progress: false`;
- evaluator wake-ups use the distinct coalescing key
  `gate-eval:v1:<repository-id>:<target-kind>:<head-sha>`, also with
  `cancel-in-progress: false`; and
- before requesting a publisher mutation, either role obtains the same durable, fenced
  application lease `gate-mutate:v1:<repository-id>:<target-kind>:<head-sha>`. The
  isolated liveness service uses this lease too, so it cannot race an Actions invocation.
  Only the mutation service can PATCH with the App token, and it rejects a stale fencing
  token before the serialized write; and
- worker launch uses a per-lane durable CAS record keyed by
  `gate-launch:v1:<repository-id>:<target-kind>:<head-sha>:<gating-epoch>:<lane>`, not an
  Actions concurrency group. Dispatch retry may create more than one Actions run, but only
  one current claimant receives the initial target data and only the newest claim fence may
  report. A superseded claimant may finish already-started local compute after its lease is
  lost, but it has no credentialed side effect and its report is rejected.

Actions concurrency may replace one pending run with another. That is acceptable only
within a role because the retained broker invocation fully reconciles the desired generation
and the retained evaluator fully re-reads the world. An evaluator event can never evict the
only pending broker. Correctness does **not** depend on the number, ordering, replacement,
or retention of evaluator events, and liveness does not depend on retention of a particular
broker event because both sweep paths rediscover every eligible target and enqueue broker
reconciliation. No invocation applies an event as an incremental delta, and no replaced
Actions run can delete a journaled epoch, manifest, outbox row, or worker claim.

Evaluator-level coalescing is an idempotent fast path in the persisted state: record a digest
of the validated, transport-filtered observation, `gating_epoch`, timeline high-watermark,
sealed-manifest/outbox state, and next wall-clock deadline. A serialized invocation may exit
without advancing counters or writing the publisher only when that entire digest is unchanged,
all authoritative receipt sources were exhaustively read, and no deadline is due. If any part
changed, or a deadline is due, it evaluates and stores the new checkpoint in the independent
journal before mirroring it to the publisher. Extra queued invocations are therefore harmless
fast no-ops rather than a correctness assumption about platform queueing. The concurrency
primitive's role is documented by GitHub's concurrency contract:
<https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency>.

Creation must also be idempotent. Under the shared fenced lease the broker/evaluator applies
the identity and collision rules in §6.1, creates one publisher only when there is no
collision, and never silently picks a winner from duplicate required contexts. The
evaluator refuses success when the durable desired intent is absent, differs from current
authoritative target state or journaled epoch, is incomplete, has a non-terminal outbox/claim,
or names any missing worker/reporter. Publisher reopen and sibling launch follow the
generation-sealing protocol above; observing an asynchronous event is never used to claim that
stale success was revoked in time. The Checks update contract permits the App-owned run's
status to be updated, and the broker's journal/PATCH/GET ordering is covered by the adversarial
tests in §6.3.

## 7. Recommended architecture

The design separates unprivileged target work, non-verdict doorbells, the privileged
generation broker/evaluator, best-effort Actions reconciliation, and an independently
scheduled liveness path on isolated capacity. No workflow or service that executes a PR or
merge ref receives the dedicated App credential or dispatch privilege.

### 7.1 Non-verdict doorbells and broker-launched workers

A small PR doorbell runs on `pull_request_target`, so GitHub takes its definition from the
default branch. It never checks out a ref, restores a cache, downloads an artifact, or
executes a repository script. Its repository token is read-only; the only elevated permission
is `id-token: write` so it can call the authenticated external broker ingress with a fixed
audience. The ingress verifies repository id, workflow identity, exact default-branch
`job_workflow_ref`, protected environment, and nonce before treating the call as a wake-up.
Event kind and PR head SHA remain untrusted data. No doorbell receives `actions: write`,
`checks: write`, or a publisher/launch credential. Same-SHA label, readiness, reopen, and
manual automation events ring this doorbell rather than launching target work themselves.

For merge groups, the existing default-branch doorbell invokes the same broker on
enqueue/dequeue. It is the only native `merge_group` target trigger. There is no
target-ref-defined writer or dispatcher whose definition could come from the combined ref.

Separately from those Actions wake-ups, the control plane terminates the GitHub App webhook
sources named by the committed transition map and journals each immutable delivery/event id
before acknowledging it. Its reconciliation reader also paginates the live-proven GitHub
timeline/attempt sources from their stored cursors. Thus a lost Actions doorbell does not lose
the epoch receipt, and a delayed or duplicate webhook is only another idempotent reconciliation
input; a source whose delivery log cannot be replayed and cross-checked remains a cutover
blocker under §6.3.

Neither doorbell can publish or update a verdict, and neither is a durable queue. They are
latency hints only. If one is absent, fails, or its pending Actions run is replaced, the
Actions sweep and independent liveness path both rediscover the target from authoritative
GitHub state and invoke the same broker reconciliation. An implementation that requires the
original event payload to finish a generation is rejected.

The broker is a level-triggered reconciler, not an event handler. It validates the target and
timeline high-watermark, computes and durably records the epoch-bound desired intent, acquires
the shared fenced lease, and follows the idempotent transaction in §6.3. Re-entry after any
interruption — epoch receipt, publisher reopen, partial sibling creation, manifest seal,
outbox commit, dispatch ambiguity, worker claim, or reporting — converges on exactly one sealed
generation and at most one active claimant per lane. Once sealed, it asks the external launch
service to pump journaled outbox rows for worker workflows whose definitions come from the
reviewed default branch. A worker authenticates and claims its stable launch key before it
receives target data; a duplicate or stale claimant exits without checking out the target.
The winner checks out the target SHA with persisted credentials disabled and receives no
dedicated-App credential, repository write token, Actions launch token, or untrusted cache
restore. Its result is data returned to a separate trusted reporter, which matches worker run
id, claim fence, lane, repository, head SHA, gating epoch, and generation nonce against the
sealed manifest before submitting a fenced update for that one sibling row to the
publisher-mutation service.

Except for the authenticated resident `ci-summary / gate` migration anchor in §6.2, all
gating workers lose native `pull_request`, `merge_group`, `labeled`, `unlabeled`,
`ready_for_review`, and target-triggered reporter entry points. A repository test enumerates
those event forms, reusable workflows, workflow dispatches, rerun helpers, and API writers
in both directions: only non-gating doorbells and that exact guarded resident may start
natively, and only the broker may create new target-SHA gating rows. This conversion is a
prerequisite, not an optimization.

### 7.2 Default-branch evaluator

`ci-gate-eval.yml` is the best-effort Actions adapter around the shared evaluator core. It
has **no** `pull_request` or `merge_group` trigger. Its privileged job runs only on:

- `workflow_run` with `types: [requested, in_progress, completed]` and an explicit
  `workflows:` list;
- `workflow_dispatch` with a target SHA and checkpoint correlation, dispatched at the exact
  default-branch ref only by the external launch service from a journaled evaluator outbox row.

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
   and records an evaluator outbox wake-up after its last write;
4. `pr-area-label` remains an unconditional manifest row for every PR and merge-group
   target; the two default-branch doorbells' no-checkout, no-check-write, fixed-audience
   OIDC-ingress-only posture and the resident gate's sole-native exception/bootstrap guard
   are pinned.

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

The evaluator workflow token keeps only the current read set plus `id-token: write` for the
fixed-audience control-plane API. The protected publisher-mutation service issues to itself a
short-lived installation token scoped to the Sparq repository id with `checks: write` only;
the installation-level `statuses: write` eligibility permission is not included. The Actions
job receives neither token nor minting key. It uses the evaluator coalescing key and then the
shared fenced mutation lease from §6.3, validates/loads the independent checkpoint, removes
exactly the trusted publisher row per §6.1, performs one `step()`, repeats the complete
identity, timeline high-watermark, epoch-bound desired intent, manifest/outbox, and generation
read before success, and submits the fenced `gate-shadow` or (only after cutover) `gate`
mutation to that service.

Workflow dispatch is isolated in a different external launch service. Its short-lived,
repository-scoped GitHub credential has `actions: write`, which GitHub also permits to rerun or
cancel runs and enable or disable workflows; describing it as dispatch-only would be false.
The service is therefore treated as a high-privilege component even though its own API accepts
only a versioned launch key naming an allowlisted worker/evaluator workflow id, exact reviewed
default-branch ref, sealed epoch/generation, current fence, and pending outbox row. It has no
free-form repository, ref, workflow, run-control, or enable/disable endpoint. It writes an
append-only audit record before and after every GitHub call, refuses target-controlled claims,
and is the only holder of the launch credential. No repository Actions job — doorbell,
sweeper, evaluator, worker, or reporter — has `actions: write` or can mint that token.
GitHub's workflow and workflow-run endpoint contracts are the authority for that permission
surface:
<https://docs.github.com/en/rest/actions/workflows>
and <https://docs.github.com/en/rest/actions/workflow-runs>.

### 7.3 Default-branch sweep

`ci-gate-sweep.yml` runs at GitHub's minimum supported schedule interval and on manual
dispatch. The schedule is explicitly best-effort: configured cadence makes no promise about
start time or eventual delivery. The workflow holds read permissions plus `id-token: write`
for the fixed external broker ingress; it does **not** hold `actions: write`, `checks: write`,
the launch token, the App token, or either minting credential.

Each sweep enumerates **all** current non-draft PR heads and active merge-group heads,
including targets with no publisher, then dispatches broker reconciliation at the exact
default-branch revision once per repository + target kind + SHA through that ingress. It also
enumerates non-terminal publishers and same-name identity failures. An identity failure raises
an alert and remains
a broker target so the durable neutralization rules in §6.1 run; it is never silently
skipped. The broker exhaustively advances timeline receipts, performs a full desired-state
read, repairs the journal/epoch/generation/outbox/claims, and requests evaluation only after
the manifest is sealed. Broker and evaluator use the distinct concurrency keys and shared
lease from §6.3; launch workers use the separate outbox CAS rather than either Actions queue.

`pr-area-label` is the one unconditional expected source workflow per target. The
evaluator records the target discovery time and refuses a stable-empty success until that
run is observed successful and the existing startup/settle semantics are satisfied. If
it never registers or never becomes terminal by the corresponding wall-clock deadline,
either reconciliation path drives `gate-shadow`/`gate` to a terminal failure. A total
Actions outage may suppress this best-effort sweep, but the missing required context remains
fail-closed and the independent path below still owns the bounded diagnostic.

### 7.4 Independent bounded-liveness driver

The wall-clock guarantee is implemented by a separately scheduled control-plane service,
not `ci-gate-sweep.yml`. Its scheduler and compute do not depend on the affected repository's
hosted-runner queue; alternatively, the compute may use separately reserved capacity only
after a saturation test proves the same isolation. The service runs a pinned build of the
same reviewed broker/evaluator core, validates that the build belongs to reviewed
default-branch history, and never checks out or executes a target ref. It uses the shared
publisher-mutation and external launch services; only the former obtains the
repository-scoped checks-only runtime token described in §4, and only the latter obtains the
broad Actions token described in §7.2.

The scheduler starts cycles at a measured maximum interval chosen so worst-case start jitter
plus one full supported-envelope sweep fits inside `LIVENESS_SLO`. Each cycle enumerates the
same complete target set as §7.3, advances every timeline high-watermark, repairs due
outbox/claim state, performs broker reconciliation, and evaluates every due wall-clock deadline
under the shared fenced lease. Duplicate, reordered, or simultaneous Actions/control-plane
ticks are idempotent. If the service misses its start bound or SLO, cannot acquire fresh target
state, loses publisher/launch authentication, cannot reconcile due outbox/claims, or cannot
durably update a publisher, an out-of-band monitor pages and admission pauses; it never converts
its own outage into success.

That bound is valid only inside a versioned **supported-capacity envelope** generated from
Phase-5 structured load evidence. The envelope records, without copying benchmark values into
this design record, the maximum eligible-target population, targets due in one cycle,
check-run and timeline pages per target, API calls per reconciliation, primary-rate budget
floor, bounded secondary-rate-limit backoff, per-head lease wait, pending outbox/claim backlog,
isolated control-plane concurrency, cold-start bound, and publisher latency. The proof artifact
must show, at those maxima, that one independent sweep satisfies
`T_start_jitter + T_discovery + T_due_reconciliation + T_lease_wait + T_outbox_repair +
T_publish ≤ LIVENESS_SLO`, with each term derived from the same structured trial rather than
an optimistic average.

Before accepting a new target, and again at the start and end of every sweep, the admission
controller compares live target/backlog counts, pagination, isolated capacity, and GitHub's
primary/secondary rate state with that signed envelope. Reaching an enumeration cap, dropping
below the reserved API budget, exceeding the bounded backoff, or placing even one target beyond
the supported backlog pauses new admission and pages out of band. Existing targets remain
fail-closed; the system explicitly makes no bounded-liveness claim outside the envelope. It may
resume only after a complete in-envelope sweep and fresh exact-head evaluation, never merely
because the backlog counter later falls.

Phase 5 must demonstrate this path while `workflow_run` and Actions schedules are suppressed
and the hosted-runner pool is saturated. Tests exercise every dimension exactly at its derived
boundary, then one unit beyond it; exhaust the reserved primary budget, inject the maximum and
over-maximum secondary backoff, fill the due/outbox backlog, and cold-start the isolated
service. At the boundary one sweep must reconcile every due target and a genuine hang must
become RED by its semantic deadline plus `LIVENESS_SLO`; every over-boundary case must pause
admission and refuse success. Failure is a cutover no-go, not a reason to relax the deadline,
capacity envelope, or branch protection.

### 7.5 Independent emergency recovery plane

Removing the resident waiter also requires a recovery plane that survives loss or revocation
of the dedicated publisher App and its mutation service. A workflow merely left inert on
`main`, or a copy of the failed publisher's check-run envelope, is not sufficient. Before
cleanup, a separately deployed recovery controller must have its own durable target journal,
timeline cursor and gating epoch, sealed manifest/outbox/claim state, worker-launch and result
reporting path, evaluator checkpoint, minting boundary, and **distinct recovery App
integration** (or another source identity the ruleset can enforce). It may reuse the reviewed
pure verdict code by pinned artifact digest, but it may not depend on the primary App,
publisher-mutation service, primary journal availability, or historical primary `gate` rows.
The recovery installation follows the same permission split as §4: installation eligibility
has `checks: write` plus `statuses: write` only if live source selection requires it, each
publisher runtime token is repository-scoped and `checks: write` only, and any recovery
worker-launch token with technically broad `actions: write`
authority is confined to its own external allowlisted launch service rather than an Actions
job.

When activated under a protected out-of-band switch, the recovery controller keeps ordinary
admission paused, enumerates authoritative current PR heads from scratch, and creates a fresh
recovery epoch and generation for each. It launches and claims every required lane through its
own fenced outbox, accepts only recovery-generation reporter receipts, computes the verdict
from freshly enumerated evidence, and publishes a new exact-head `gate` with the distinct
recovery integration. A fresh merge-group canary likewise starts with newly discovered target
state and a new recovery generation; it cannot assume that the primary broker ever sealed one.
Only after all current heads and that canary are freshly terminal may a full ruleset projection
select the recovery integration and a tightly controlled canary admission reopen.

If no distinct recovery integration or enforceable equivalent exists, the on-`main`
App-15368 transport is emergency diagnostic machinery only. Because any target-controlled
Actions job shares App 15368, its same-name result cannot prove provenance strongly enough to
satisfy the recovered rule. It may collect evidence while admission remains paused, but it may
not authorize a ruleset switch, merge, or reopening. That condition is a Phase-7 no-go.

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
- **The isolated control plane has real cost and operational surface.** Its scheduler,
  fenced lease, credential broker, monitoring, and recovery transport belong in the
  measurement. They are correctness dependencies, not optional optimizations that may be
  removed to improve the headline saving.

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
   inventory gate, durable collision handling, the independent append-only target journal,
   fenced per-head mutation lease, the resident sealed-manifest bootstrap guard in dormant
   observe-only mode, and multi-invocation adversarial tests (§6). Provision the dedicated
   publisher App, the exclusive fenced publisher-mutation/minting service,
   installation-level `checks: write` plus source-selection `statuses: write`, and
   repository-scoped checks-only runtime tokens.
   Preserve the resident driver's existing `SELF_RUN_ID` ordering and tests unchanged
   outside the dormant guard.
4. **Phase 3 — broker the launch topology.** Add the generated wake-up/launch inventory,
   its both-direction drift test, the default-branch PR and merge-group OIDC doorbells,
   monotonic gating epochs and timeline receipts, sealed sibling manifests, epoch-bound
   desired digests, durable dispatch/evaluator outboxes, worker-side fenced claims, distinct
   broker/evaluator concurrency keys, level-triggered reconciliation, unprivileged worker
   execution, and trusted result reporters (§6.3–§7.3). Provision the authenticated external
   launch service; record that its repository-scoped `actions: write` credential can dispatch,
   rerun, cancel, enable, and disable workflows, expose only the allowlisted fixed-ref launch
   API, and prove no Actions job can obtain it. Exercise all of this first on de-armed canaries
   while old sibling triggers remain. Inject lost doorbells, A→B→A same-SHA history, replaced
   pending brokers, dispatch-response loss, duplicate workers, expired claims, and crashes at
   every journal/GitHub boundary; every later sweep must converge with no stranded lane and at
   most one active worker/reporter claim. Once that path is verified, seed every eligible
   head, request and verify fresh non-terminal resident attempts, and activate the bootstrap
   guard. Only then remove every native target trigger from gating workers, including same-SHA
   label/readiness and rerun helpers, except the exact authenticated resident gate. That
   guarded resident remains authoritative while this large migration is exercised; failure
   to achieve complete broker ownership chooses defer/reject rather than partial cutover.
5. **Phase 4 — shadow transport.** First merge and verify the resident driver's strict
   dedicated-App transport exclusion under both `gate-shadow` and migration `gate` names;
   only then activate the evaluator, best-effort Actions sweep, and independently scheduled
   isolated-capacity liveness driver. The dedicated App publishes the non-required
   `gate-shadow` context, excludes authenticated legacy gate transports, and exercises PR,
   fork, initial run, broker-sealed rerun, no-leg, cancellation, genuine-hang, late
   reporter, merge-group, same-SHA epoch replacement, publisher reopen, outbox/claim recovery,
   wrong-App/name collision, pagination/truncation, and duplicate-publisher cases.
6. **Phase 5 — evidence gate.** Compare every terminal shadow verdict with the resident
   verdict and measure wake/terminal latency. Audit the complete reserved-name and
   credential inventory; read back installation permissions, checks-only publisher tokens,
   broad-but-isolated launch-token authority, and ruleset source selection; prove generation
   sealing, timeline-epoch ABA resistance, outbox/claim crash recovery, and that the two
   transports never observe one another. Generate the signed supported-capacity envelope from
   structured trials. Suppress `workflow_run` and Actions schedules, saturate the hosted pool,
   and prove one isolated sweep reconciles every due target at each exact envelope boundary,
   including a lost initial doorbell and genuine hang, within `LIVENESS_SLO`. Exercise one
   unit over every target/backlog/pagination/API-budget/backoff/lease/outbox/cold-start bound
   and prove admission pauses without success. Kill the isolated path and prove the same
   out-of-band response. Then, while the resident Actions `gate` remains required, migrate
   each non-draft PR head's dedicated-App publisher from `gate-shadow` to `gate` using the
   single-row protocol in §6.2 and repeat parity collection by check id/App. Any unexplained
   mismatch, duplicate, missing target, stale-success window, writer drift, dependency cycle,
   envelope overrun, liveness miss, or permission failure blocks cutover.
7. **Phase 6 — controlled integration cutover.** Drain the merge queue and pause admission.
   Prove every eligible PR head has exactly one current successful dedicated-App `gate`
   and one authenticated resident result. With explicit administrator authorisation,
   apply a full ruleset projection that changes only the required `gate` integration from
   GitHub Actions App 15368 to the dedicated publisher App; the required context name and
   every other rule remain unchanged. No branch-protection bypass or code commit is needed.
   Evaluate a canary PR and fresh merge-group head before reopening admission. If the
   dedicated integration cannot be selected, the runtime token can write statuses, the
   launch credential is reachable from Actions, broker/outbox recovery or the measured
   in-envelope liveness SLO is unproven, or the fenced lease is unavailable, stop: the design
   is not safe to cut over.
8. **Phase 7 — cleanup.** Cleanup is forbidden until the independent recovery plane in §7.5
   is deployed and a no-bypass drill proves its own journal, epoch, worker-launch outbox,
   claims, reporting, evaluator, and distinct recovery App/integration work after the primary
   App and mutation service are disabled. Its reviewed activation adapter and the inert
   App-15368 emergency diagnostic transport must already be merged on `main`; while dormant
   neither emits a check named `gate` or occupies a resident polling slot, and their controls
   are unavailable to target code. Without consulting the primary publisher or journal,
   exercise fresh recovery generations on PR, fork, same-SHA replacement, and a newly created
   merge-group canary, including rejection of historical green. Only the distinct recovery
   integration may support a ruleset switch or canary admission; App 15368 remains diagnostic
   and admission-paused. If that identity or independent data path is absent, cleanup is a no-go.
   Only after the drill may the active resident polling transport be removed and
   `ci-summary.yml`'s doctrine header and `docs/branch-protection.md` updated. An off-main
   rollback commit does not satisfy this precondition.

**Rollback.** Before Phase 6, disabling the shadow has no merge effect. During the Phase-6
canary window the resident Actions gate stays live for comparison, but its App-15368/name pair
is not promoted as safe recovery provenance. A cutover failure pauses admission and drains
active merge groups. The primary dedicated path is repaired in place, or the already-proven
distinct recovery plane is activated; absent either, admission remains paused. Restoring App
15368 may aid emergency diagnosis but cannot justify a merge or reopening because branch
protection cannot distinguish it from a target-controlled same-App/name row.

After Phase 7, recovery never begins with a merge through the component being recovered.
Keep admission paused and drain active merge groups, then disable/revoke the primary path and
activate the already-on-main adapter plus independent recovery controller through their
protected out-of-band switch. The controller enumerates targets without primary state, creates
fresh recovery epochs, manifests, outbox rows, claims, reporter receipts, and evaluator
checkpoints, and GET-verifies an exact-head `gate` from the distinct recovery integration for
every eligible PR and fork. It then admits only a designated fresh merge-group canary and
builds that target's recovery generation from scratch. Historical primary/recovery green,
App-15368 results, and target-controlled producers are never accepted. Only after those fresh
results may a captured full ruleset projection change solely the required integration from
the primary App to the distinct recovery App. A missing current result, undrained group,
failed recovery canary, dependency on primary state, or unexpected ruleset diff blocks the
switch. App 15368 remains diagnostic with admission paused; it is never the recovered trust
anchor. No branch-protection bypass or recovery commit is required.

## 10. Decisions and live questions

Resolved by this revision:

- `workflow_run` uses a complete explicit workflow list; no wildcard is assumed.
- PR/merge-ref execution is unprivileged and separate from the default-branch evaluator.
- every wake-up is idempotent and keyed by repository + target kind + head SHA; broker
  reconciliation is level-triggered and cannot be evicted by evaluator coalescing;
- Actions schedules are best-effort reconciliation only; the hang-to-RED bound belongs to
  an independently scheduled path on isolated capacity and is conditional on a measured,
  signed supported-capacity envelope, with failure or overload as a cutover no-go;
- every gating-affecting transition has an immutable journal receipt and advances a monotonic
  epoch that is part of the desired digest, so an unchanged SHA and A→B→A snapshot cannot
  reuse an earlier green;
- worker launch uses a durable fenced outbox and worker-side claim; an ambiguous dispatch may
  be retried, but only the current claimant receives work and only the newest fence can report;
  every stranded lane is rediscovered by a sweep;
- the unique fully validated publisher is excluded before any verdict transformation,
  while every identity mismatch or duplicate fails closed;
- App/external-id matching is not treated as authorization: a dedicated publisher App and
  a machine-pinned inventory of every reserved-name and credential path are part of the
  trust boundary;
- installation eligibility (`checks: write` plus source-selection `statuses: write`) is
  distinct from repository-scoped runtime authority (`checks: write` only);
- `actions: write` is accurately treated as authority to dispatch, rerun, cancel, enable, and
  disable workflows; only the external allowlisted launch service holds it, never an Actions
  job;
- resident and shadow transports are mutually excluded and compared out of band;
- replacement work is generation-sealed only after the required publisher has been
  reopened and GET-verified; before integration cutover a fresh resident attempt is also
  verified non-terminal, and asynchronous events provide liveness, not ordering;
- shadow publication precedes any required-context change;
- initial registration and a missing seed have explicit fail-closed behaviour and are
  repaired by both level-triggered sweep paths;
- Phase-7 cleanup retains an inert, already-on-main App-15368 emergency diagnostic transport,
  but its same-App/name result is never recovery provenance or a reason to reopen admission;
  and
- post-cleanup recovery owns independent state, launch, reporting, evaluation, and a distinct
  enforceable publisher integration, so it can build fresh PR and merge-group generations
  without the failed primary path or a recovery commit.

Phases 4–7 must answer with live evidence, not maintainer guesswork:

1. Does `workflow_run` deliver both requested/completed wake-ups for merge-group-triggered
   source workflows, with the merge-group head SHA?
2. Can the dedicated publisher App installation, with the separately documented
   source-selection permission, be selected as the required-check integration for
   same-repository, fork, and merge-group heads; is each runtime token demonstrably
   repository-scoped and checks-only; and is minting authority unavailable to all
   target-controlled workflows?
3. Which default-branch reporters need an explicit evaluator doorbell because their own
   `workflow_run.head_sha` is the default-branch SHA rather than the head they annotate?
4. Is peak concurrency or total slot time the binding constraint after the label-router
   and merge batching changes land?
5. Does one independent sweep meet `LIVENESS_SLO` at every exact target/backlog/pagination/
   API-budget/backoff/lease/outbox/cold-start envelope boundary with Actions suppressed and
   hosted runners saturated, and does every over-boundary or driver failure page and pause
   admission without publishing success?
6. Does a missed doorbell, same-SHA A→B→A sequence, replaced pending broker, dispatch-response
   loss, duplicate worker, expired claim, or crash at each broker transaction boundary
   converge from a later sweep with a later epoch, one sealed generation, no stranded lane,
   and at most one active claimant?
7. After the primary App, journal, and mutation service are unavailable, can the distinct
   recovery plane discover targets from scratch, create fresh PR and merge-group generations,
   publish exact-head results under its own integration, and apply the captured ruleset
   projection without a commit or bypass while App 15368 remains diagnostic-only?

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

Every implementation PR must add machine assertions for the corresponding phase. Across
the complete chain, those assertions must prove:

- **Liveness:** with `workflow_run` and Actions cron suppressed and hosted capacity fully
  occupied, one isolated sweep drains every due target at each exact structured-envelope
  boundary and evaluates a fake-clock deadline within `LIVENESS_SLO`. A one-unit overload,
  exhausted reserved API budget, excess secondary backoff, or stopped driver raises the
  out-of-band incident, pauses admission, and cannot publish success.
- **Epoch safety:** for every supported readiness/label/state/queue/rerun/inventory mutation,
  the durable receipt and epoch advance precede or are atomic with a broker-origin GitHub
  effect, and precede the admission-visible effect of an external proposal. With both doorbells
  dropped, an A→B→A same-SHA history still advances the epoch twice, reopens prior success, and
  rejects the old reporter; a receipt gap or truncated timeline fails closed.
- **Reconciliation:** dropped doorbells, pending-run replacement, duplicate/reordered wake-ups,
  and crashes after epoch receipt, publisher reopen, partial row creation, manifest seal,
  outbox commit, accepted dispatch, claim, and report all converge from a later sweep. The
  evaluator cannot succeed before the epoch-bound intent is sealed and every outbox row is
  terminal. Ambiguous dispatch retries leave no lane stranded, concurrent duplicate workers
  receive no target data, and a superseded claim cannot report.
- **Authority:** live installation readback contains the source-selection permissions; a
  repository-id-scoped checks-only token can create/update a check but receives a denial for
  commit-status write; the mutation service is the only holder, and no Actions adapter, PR,
  merge-ref worker, or target artifact can reach the App token or minting authority. No Actions
  job has `actions: write`; live denial tests prove its OIDC identity cannot invoke arbitrary
  launch-service operations, while the service audit shows its technically broad token is used
  only for allowlisted fixed-ref launches. The services reject a stale fence, arbitrary check
  or workflow name, wrong repository/head/check id, run-control/enable/disable request, illegal
  transition, and success without a complete sealed-generation proof.
- **Recovery:** after disabling or revoking the primary publisher and withholding its journal,
  the independent recovery plane builds fresh state, epochs, manifests, outbox/claims,
  reporter receipts, and verdicts for same-repository, fork, same-SHA replacement, and a newly
  discovered merge-group canary. Only its distinct integration can become required; historical
  green and every App-15368 result are rejected for recovery admission. Normal mode emits no
  recovery `gate`, consumes no resident polling slot, and the ruleset switch changes only the
  required integration without a commit or bypass.

Still not verified: dedicated/recovery-App installation and ruleset selection, reduced
publisher-token scope, isolation of the broad launch token, merge-group wake-up payloads,
timeline-receipt completeness, fenced-lease/outbox/claim behaviour, loss-recovering broker
reconciliation, the supported-capacity envelope and independent-path latency under saturation,
recovery-plane activation from no primary state, required-check registration and
generation-sealing timing, exhaustive check-run enumeration at live scale, and shadow/live
verdict equivalence. Phases 4–5 turn the primary-path assumptions into evidence before the
required integration changes; Phase 7 adds the independent recovery drill before active
resident cleanup.
