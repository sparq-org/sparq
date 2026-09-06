# Unit occupancy: why de-duplicating reservations cannot widen the frontier

> 🤖 SPARQ agent — measured design record. Snapshot: `sparq-org/sparq`, 2026-07-27,
> 1473 open issues / 123 open PRs, cursor-paginated (`gh api --paginate`, page counts
> cross-checked against the listing totals).

## The claim under test

The dispatch frontier is narrow while the drainable backlog is large. One proposed cause: a
worker PR and its source issue are the **same unit of work** and each reserves its `area:`
partitions independently, so the same work is counted twice. The proposed remedy was to
**deduplicate** — reserve the union once — explicitly *not* to drop either half.

## What the code actually does

`scripts/ready-issues.py::compute_ready` reserves for a row iff it is OPEN, not parked
(`needs:user` / `review:needs-user` / `status:blocked`), and is either a PR row or carries
`status:in-progress` / `status:in-progress-review`. Census over the live snapshot, taken from that
code path rather than from a description of it:

| population | count |
| --- | --- |
| open PRs, occupying, with an `area:` label → reserve | 65 |
| open PRs, parked → reserve nothing | 49 |
| open PRs, occupying, no `area:` → reserve nothing | 9 |
| in-flight issues with an `area:` label → reserve | 46 (44 `in-progress-review` + 2 `in-progress`) |
| **total** | **111 artifacts, 158 reservations, 49 distinct partition keys** |

Parked PRs contribute **zero**. 20 of the 158 reservations are a duplicate of a key the unit's
other half already holds.

## Finding 1 — deduplication is frontier-neutral, provably

`conflict()` tests membership in the **set** of held partition keys (`blockers.keys()`). A second
occupant on an already-held key appends to a list that only attribution ever reads. And a unit
reserves the union of exactly its members' own reservations, so the union *over units* equals the
union *over members*. Therefore the held key set — and hence the frontier — is **invariant** under
folding. Measured, on the live snapshot:

| occupancy | artifacts/units | reservations | distinct keys | frontier |
| --- | --- | --- | --- | --- |
| both halves, independent (status quo) | 111 | 158 | 49 | 3 |
| union folded into one unit each | 81 | 138 | 49 | **3** |

Dedup recovers **zero** slots. Anything that *does* widen the frontier does so by **releasing** a
key. `test_the_held_key_set_is_invariant_under_folding` pins this so no future frontier assertion
is written believing it can witness the fold.

## Finding 2 — the issue half cannot be dropped: 41% of pairs are not supersets

Over the 94 open PRs with at least one open linked source issue (same-repo
`sparq-agent/issue-N-*` head, or a closing keyword from a trusted association):

| relation of PR's `area:` set to its source issue's | pairs |
| --- | --- |
| PR ⊋ issue | 31 |
| identical | 18 |
| source issue declares no `area:` at all | 6 |
| **PR ⊊ issue** | **13** |
| **incomparable — each declares a key the other lacks** | **26** |

So in **39 of 94 pairs (41%)** the PR's file-derived key set is *not* a superset, and dropping the
issue's reservation would free a key the unit really occupies. Under-serialisation is the
corrupting direction: two workers in one crate produce semantic conflicts that compile, pass, and
are invisible to git. The union is the only rule safe in both directions, and it is monotone by
construction — a unit's reservation is a superset of every member's own.

Registry CLAIM's extra fail-closed step (`areas |= issue_areas or {GLOBAL_PACKAGE}`) is **not**
adopted here. Applied to the linked population it drives the live frontier to **0** — the same
whole-fleet seizure `_reserving_packages` exists to prevent.

## Finding 3 — the real defect is the opposite: both layers under-hold

The two layers that consume this engine hold **different, incomparable subsets** of the union:

| view | distinct keys held | frontier |
| --- | --- | --- |
| local CLI (`dispatchable_view`, PR rows retained) | 49 | 3 |
| registry `dispatch.yml` PLAN (PR rows **stripped**) | 37 | 9 |

`dispatch.yml` builds its readiness input as
`[issue for issue in snapshot("issues", index) if "pull_request" not in issue]`, so PLAN reserves
the issue half of every unit and never the PR half — 12 keys held here are free there
(`ci`, `deps`, `release`, `sparq-algos`, `sparq-core`, `sparq-e2ee-ng`, `sparq-engine`,
`sparq-geo`, `sparq-reason`, `sparq-substrate`, `sparq-trust`, `zk`). **7 of PLAN's 9 frontier rows
land in a key an open PR already holds.**

They are not double-dispatched — CLAIM's `busy_packages_of_pulls` re-derives busy areas from the
pulls snapshot and drops them — but they are dropped *after* `compute_ready` committed the
frontier, so each burned a partition with no backfill. That is the registry's own issue #113 shape,
one layer up. The counterfactual "+N frontier from dropping the PR half" is therefore not
recoverable capacity: it is a measurement of serialisation the pipeline is doing on purpose.

## Where the fix binds

The occupancy **definition** is sparq's (`unit_reservations`). The **input** to it on the live
dispatch path is built inside `dispatch.yml`, which the registry owns, and sparq has no hook into
it — PLAN is deliberately token-less and cannot fetch pulls itself. So:

* sparq ships the shared, pure definition plus `occupancy_parity`, which reports the gap loudly on
  every `--diagnose` run instead of it being re-derived by hand.
* The binding change is registry-side: stop discarding PR rows, and pass `source_links`. The
  `source_links=None` default makes sparq's half a **no-op** for the existing registry call —
  verified byte-for-byte on the live snapshot, frontier rows and all 347 conflict-attribution lines
  identical — so the two repositories may merge in either order.
