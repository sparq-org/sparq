# Batch-merge v2: from-the-start revision pinning (design record)

> 🤖 SPARQ agent — design record for target issue **#3490**. DESIGN ONLY: no behaviour
> change ships with this record. It re-approaches the batch-merge v2 program (culprit
> bisection + the follow-on throughput ideas from PR #3435, held `needs:design`) with
> revision pinning as the *foundational invariant* rather than a patch applied race by
> race. v1 is `scripts/batch-merge.py` + `.github/workflows/batch-merge.yml`; read the
> v1 header first — everything there (pure `plan(state) -> [Action]`, hermetic
> self-test, App-token rules, class partition, caps, hygiene-only mode) is KEPT.

## 1. Why iterative patching lost: one race class, many faces

PR #3435 (uncapped omnibus + recursive bisection) went four review rounds, and each
round surfaced a *new instance of the same defect class*: the state machine read
**mutable branch refs at different times** and then acted as if those reads described
one consistent world. Concretely surfaced instances:

1. **Wrong-conviction of a sibling after a force-push.** Bisection built a probe
   sub-batch by re-fetching constituent branches. A constituent force-pushed between
   the failing batch and the probe, so the probe evaluated *different content* than the
   evidence run — an innocent sibling could inherit the blame (or a real culprit walk).
2. **Landing a superseded revision.** The eligibility read (`review:pass` + armed) and
   the `git fetch` + `merge origin/<branch>` happened at different times. A worker
   pushing between them means the omnibus can land a revision nobody reviewed — or an
   old revision the worker had already superseded — while the constituent's issue
   closes via the omnibus `Closes #` refs.
3. **Branch-delete TOCTOU.** Stale hygiene deleted any `sparq-omnibus/*` branch "with
   no open PR" based on a PR-list read taken earlier in the run. A branch (re)created
   between the read and the delete is destroyed while live.
4. **Emptiness-vs-live drift.** Constituent closure compared the *live* branch tip
   against `origin/main`; a commit pushed mid-run flips the answer between the plan and
   the close.

Patching these one at a time converges slowly because the class is generative: every
new read site or mutation site added to the state machine mints a fresh TOCTOU. The v2
design therefore removes the class structurally.

## 2. The v2 invariant (normative)

Three rules. Every future batch-merge feature (bisection, uncapping, anything else)
must satisfy all three or it does not ship.

**R1 — SNAPSHOT ONCE.** A run performs exactly one read phase, producing an immutable
snapshot: `origin/main`'s OID, the OID of *every* ref the plan may reason about or
mutate (constituent heads, omnibus branches, probe branches), and the PR metadata.
`plan()` is pure over the snapshot and never re-reads. The single OID source of truth
is **one `git ls-remote origin` call** (refs are what mutations act through; the
GraphQL/`gh` PR list supplies metadata only — where the PR's `headRefOid` and the
ls-remote OID for the same branch disagree, the ref drifted during the snapshot itself
and that constituent is excluded from this run's plan).

**R2 — PIN IN THE ARTIFACT.** Every durable artifact the state machine later reasons
from carries the pinned OIDs, so a later run reasons about *revisions*, never live
branches:

```
<!-- sparq-omnibus:v2 class=<class> base=<main-oid> constituents=<pr>@<oid>,<pr>@<oid>,… -->
<!-- sparq-omnibus-close:v2 pr=<n> head=<oid> omnibus=<m> -->
<!-- sparq-omnibus-convict:v2 pr=<n> head=<oid> episode=<id> evidence=<run-url> -->
```

The v1 marker keeps parsing (legacy omnibus bodies must keep closing their
constituents); v1-marker constituents get v1 (live-branch) closure semantics until the
last pre-v2 omnibus drains.

**R3 — LEASE EVERY MUTATION.** Every destructive action carries an *expected-OID
precondition* checked as close to the mutation as the primitive allows — server-side
where a primitive exists, read-back immediately before otherwise. On lease failure the
action (and its dependents) are **skipped**; the run never refreshes an OID and
retries. The next scheduled run re-plans from a fresh snapshot. Drift is not an error:
it is the normal signal that the world moved, and the response is always
skip-and-re-plan.

## 3. Mechanism catalog: a lease per mutation

| Mutation | v1 primitive | v2 leased primitive | Lease strength |
|---|---|---|---|
| Fetch constituent content | `git fetch origin <branch>` | fetch the branch, then require the pinned OID to be present (`git cat-file -e <oid>^{commit}`); absent ⇒ drift ⇒ skip constituent | exact (content-addressed) |
| Merge constituent into omnibus | `git merge --no-ff origin/<branch>` | `git merge --no-ff <pinned-oid>` — merge the *revision*, never the ref name | exact (content-addressed) |
| Create omnibus/probe branch | `git push origin <branch>` | unchanged — a plain (non-force) push of a new ref is an atomic create that fails if the ref exists | server-side |
| Arm / merge a PR | `gh pr merge --auto` | `gh pr merge --auto --match-head-commit <head-oid>` (repo precedent: `scripts/check-pr-arm-base.py` already models this flag) — GitHub refuses the merge if the head moved | server-side |
| Delete a branch (failed omnibus, stale hygiene) | `git push origin --delete <branch>` | `git push --force-with-lease=refs/heads/<branch>:<pinned-oid> origin :refs/heads/<branch>` — the server rejects the deletion unless the remote ref still equals the pin | server-side |
| Close a constituent PR | `gh pr close` after live emptiness test | emptiness computed vs the **pinned** OID (`git merge-tree --write-tree origin/main <pinned-oid>` equals main's tree), AND a read-back of `headRefOid == pin` immediately before the close; the close comment records the pin (`sparq-omnibus-close:v2`) | read-back + reversible (§5) |
| Quarantine a convicted constituent (v2 bisection: draft + disarm) | — (new in v2) | conviction binds to the **OID**, not the PR: read-back `headRefOid == convicted-oid` immediately before drafting; the convict comment records the pin | read-back + reversible (§5) |

Two primitives (`close`, `draft`) have no server-side precondition on GitHub. The
residual window there is one read-back-to-mutation round-trip — small, but nonzero, and
this design does not pretend otherwise. Both mutations are chosen to be *reversible*,
and §5's reconciliation leg makes the residue self-healing instead of silent.

## 4. Bisection over pins (what makes v2 sound)

A batch is now a value: `B = (M, [(pr₁,o₁), …, (prₙ,oₙ)])` where `M` is the pinned
`origin/main` OID and each `oᵢ` a pinned constituent head. Failure evidence binds to
`B`, and every rule below compares against the pins, never a live branch:

- **Probes are pure functions of `B`.** Every probe sub-batch in a bisection episode is
  built from the SAME `M` and the SAME `oᵢ` recorded in the failing omnibus's v2
  marker — never refetched from branch names. All probes therefore evaluate subsets of
  one fixed world, which is the property recursive bisection needs to be a search at
  all. (This kills #3435 race 1 by construction: a force-push cannot leak into a
  probe, because probes never read branches.)
- **Causal exclusion is episode-scoped.** "This half passed, so its members are not the
  culprit" is valid only within the episode's frozen `(M, pins)`. It prunes the search;
  it is *not* a certificate that those revisions are green on current `main`.
- **Conviction names a revision.** The outcome is "PR #n *at OID o* breaks the batch on
  base M", recorded in the `sparq-omnibus-convict:v2` marker with the evidence run URL.
  The quarantine action (draft + disarm, so the registry's review lane routes it back
  to its worker) executes only if the live head still equals `o`. If the worker already
  pushed past `o`, the conviction is **void** — the evidence describes a superseded
  revision — so the action is skipped and nothing is filed against the new revision.
- **Innocents land through the front door.** Exonerated constituents are NOT merged
  from the episode's frozen base; they simply remain individually armed and re-enter
  the ordinary v2 omnibus path, whose next run snapshots fresh `M` + fresh pins and
  whose gate re-validates them on current `main`. Bisection only ever *removes* a
  constituent from circulation; it never lands code. This keeps the episode's frozen
  base sound (nothing stale can reach `main` through it) and keeps the landing path
  single (one code path to trust).
- **Episode state is stateless-reconstructable.** Probe branches are
  `sparq-bisect/<episode>-<range>`; probe PR bodies carry the episode + pins marker.
  Like v1, any run can reconstruct the full episode from markers — the runner keeps no
  state between runs, so a crashed run degrades to a re-plan, never a wedge.

## 5. Drift responses and the reconciliation leg

Every lease failure has one response — skip + re-plan — but two mutations need a
*repair* leg because their lease is only a read-back (§3):

- **Close reconciliation.** Each run, for every constituent closed by a
  `sparq-omnibus-close:v2` comment: if the PR's current head no longer equals the
  recorded pin (the worker pushed inside the residual window, or after the close), the
  run reopens the PR and restores its arm. The recorded pin makes this deterministic —
  no heuristics about "did we close it wrongly".
- **Conviction reconciliation.** Each run, for every open drafted PR carrying a
  `sparq-omnibus-convict:v2` marker: if the current head has moved past the convicted
  OID, the new revision is presumed innocent — the PR is un-drafted and re-armed (it
  re-enters the normal path; a fresh batch failure would convict the *new* OID with
  fresh evidence).

Both legs are idempotent and run in hygiene-only mode too (they mutate PR state, not
refs, so they work without the App token — same posture as v1's closure leg).

## 6. What stays exactly v1

- The **pure-plan architecture**: `plan(snapshot) -> [Action]`, actions carrying exact
  argv, executed by a thin runner; `--self-test` with `gh` + `git` stubbed. v2 extends
  `Action` with an `expected_oid` field the runner enforces; that is the whole delta to
  the execution model.
- **Class partition, QUEUE_WINDOW, MIN/MAX_CONSTITUENTS, the age bound, re-arm, App
  token requirements, hygiene-only degradation.** The #3435 "uncapped omnibus" idea is
  explicitly out of scope here: it is orthogonal to pinning, it interacts badly with
  bisection depth, and per the maintainer's note on #3490 the queue's current window
  drains fine — revisit after the review-lane throughput work, on top of pins.
- **Non-gating posture.** batch-merge stays off the required-checks set
  (`docs/branch-protection.md`); nothing here touches `ci-summary / gate`.

## 7. Test plan (self-test extensions, all hermetic)

Fixtures pin exact argv **including OIDs**, so any drift in lease construction flips
the suite red:

1. Plan-shape: v2 marker emitted with `base=` + `pr@oid` pins; merges target OIDs, not
   `origin/<branch>` refs.
2. Leased delete argv: `--force-with-lease=refs/heads/<br>:<oid>` on both the failed-
   omnibus and stale-hygiene paths; a fixture where the snapshot OID differs from the
   marker pin must still lease on the *snapshot* OID (the ref-truth at plan time).
3. Leased arm argv: `--match-head-commit <oid>` present on every arm, including §7-v1
   re-arms.
4. Drift-at-fetch: pinned OID absent after fetch ⇒ constituent skipped, batch
   re-templated (reuses v1's conflict-skip machinery).
5. Close/convict read-back mismatch ⇒ action skipped, no comment posted.
6. Reconciliation: closed-with-pin + moved head ⇒ reopen + re-arm argv; drafted-with-
   conviction + moved head ⇒ ready + re-arm argv; unmoved heads ⇒ no action.
7. Episode purity: a bisection probe plan built from a v2 marker never emits a fetch of
   a constituent *branch name* — only pinned OIDs.
8. Legacy: v1 markers still drive v1-semantics closure; mixed v1/v2 state plans
   correctly.

## 8. Rollout

- **v2.0 — pins only.** Ship R1–R3 in the existing (non-bisecting) batcher: v2 marker,
  OID merges, leased deletes, leased arms, pinned emptiness, close read-back +
  reconciliation. This is a strict safety upgrade with no behavioural surface change,
  and it is the prerequisite the issue title names ("from-the-start").
- **v2.1 — bisection.** Only after v2.0 soaks: episode machinery (§4), conviction +
  reconciliation (§5). Every v2.1 review question should reduce to "which pin does this
  compare against?" — if a proposed step has no pin to compare against, it violates R1
  and gets redesigned, not patched.

## 9. Honest limits

- `gh pr close` / `gh pr ready` have no server-side expected-OID parameter; §3's
  read-back shrinks but cannot eliminate that window. The design compensates with
  reversibility + reconciliation (§5), not with a claim the race is gone.
- `--force-with-lease` protects against a moved ref, not against GitHub-side ref
  recreation *after* a successful leased deletion; branch-name stamping (fresh
  `sparq-omnibus/<class>-<utcstamp>` per run) keeps name reuse out of the design.
- A pinned OID can become unfetchable after a force-push (server garbage collection).
  That is handled as ordinary drift (skip + re-plan), but it means a bisection episode
  can be starved by a constituent that force-pushes mid-episode; the episode then
  dissolves and the fresh revisions re-enter the normal path — correct, just slower.
