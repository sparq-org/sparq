<!-- [OPUS-5] Decomposition/design record: sub-crate dispatch parallelism. Measurement-first. -->

# Sub-crate dispatch parallelism — can we run more than one worker per crate?

**Status:** measured; design recorded; **no implementation beads created deliberately** (see §9).
**Author:** 🤖 SPARQ agent (Opus 5 architect stage), 2026-07-26.
**Question (maintainer, 2026-07-26):** can dispatch triage the *parts* of a crate an issue will
touch — tests / benchmarks / docs / code — so several workers run on one crate at once, and can we
predict the specific files up front to widen that further?

**Answer, in one line:** the maintainer's diagnosis (the crate lock is a real ceiling) is
**correct and measured**, but the proposed mechanism — region partitioning *inside* crates — is
**measured to be worth little (14.5% ceiling)**, while the same idea applied to the **non-crate
area buckets** is worth **4× more** and is where 58.3% of today's real deferrals live. Separately,
the measurement uncovered a **live under-serialisation defect**: an ad-hoc version of sub-crate
partitioning is *already deployed* via five sub-crate `area:` labels, and it silently lets two
workers into the same crate today (§5). That defect is the highest-priority item here and it is
not an optimisation.

---

## 1. What is actually serialising, measured live

`scripts/ready-issues.py` `compute_ready()` admits at most one in-flight artifact per `area:`
partition. Live snapshot taken while writing this record (`ready-issues.py --diagnose` plus a
direct `compute_ready(conflict_log=…)` run against `sparq-org/sparq`):

| quantity | value |
| --- | --- |
| open issues | 1368 |
| drainable backlog (`ready_candidates`) | 372 |
| **concurrency frontier (`compute_ready`)** | **10** |
| conflict-deferred candidates | 362 |
| distinct `area:` values across the 372 candidates | 49 |
| ready candidates carrying **no** `area:` label (→ `__global__`) | **0 (0.0%)** |

So the lock costs roughly a 37-to-1 reduction from drainable backlog to dispatchable frontier, and
even a perfect 1-per-area scheduler with an empty board would only reach ~49. The maintainer is
right that this is a ceiling.

**Corrected premise (mine, not the maintainer's).** I went in expecting the `__global__`
fail-closed default to be the dominant serialiser, because 583 of 1368 open issues carry no `area:`
label. It is not: **all 583 are excluded earlier** by the `no status:ready attestation` gate
(797 issues, 58.3% of the backlog), so zero of them reach the partition algebra. `__global__` is a
correctness backstop that is not currently binding, and §7 keeps it intact rather than tuning it.

### Where the deferrals actually come from

Attributing all 362 live conflict-deferrals to the area that held them:

| held area | deferrals | kind |
| --- | --- | --- |
| `bench` | 36 | non-crate |
| `ci` | 33 | non-crate |
| `sparq-engine` | 25 | crate |
| `gui` | 24 | non-crate |
| `site` | 21 | non-crate |
| `sparq-reason` | 19 | crate |
| `sparq-solid` | 18 | crate |
| `upstream` | 16 | non-crate |
| `sparq-core` | 16 | crate |
| `docs` | 12 | non-crate |
| `site-papers` | 11 | non-crate |
| `zk-xpath` | 11 | non-crate |

- **held by a real-crate area: 151 (41.7%)**
- **held by a non-crate area: 211 (58.3%)**

One PR, `#4318`, carries four area labels (`site`, `ci`, `sparq-e2ee-ng`, `docs`) and by itself
held **54** candidates. 84 of the 103 open PRs carry at least one `area:` label, so PR-side
occupancy is live and effective — the `_reserving_packages` asymmetry documented in
`ready-issues.py` is working as intended.

---

## 2. The separability measurement — this is what kills the simple design

**Method.** Every squash-merge commit on `origin/main` whose subject ends in `(#N)`, deduped by PR
number, with its changed-file list from `git log --name-only`. **Sample: 2167 merged PRs** spanning
the repository's full PR-carrying history (~43 days). Each changed file is classified into a
`(package, region)` cell: `crates/<c>/src`, `/tests`, `/benches`, `/examples`, `/docs` (any `.md`),
`/manifest` (`Cargo.toml`), plus non-crate pseudo-packages for `.github/`, `scripts/`, `research/`,
`skills/`, `site/`, `gui/`, `bench/`, `js/`, and a `__workspace__` cell for `Cargo.lock` and the
workspace manifest. Script kept out-of-tree (scratch); the method is reproducible from this
paragraph in a few lines of Python.

### Do PRs touch exactly one region of one crate?

| measure | result |
| --- | --- |
| merged PRs touching ≥1 real crate | 1167 / 2167 = 53.9% |
| of those, touching exactly ONE crate | 973 / 1167 = 83.4% |
| **of single-crate PRs, touching exactly ONE region inside that crate** | **353 / 973 = 36.3%** |
| of single-crate PRs, touching `src` **and** `tests` together | **414 / 973 = 42.5%** |

Region-set distribution within the single crate (top rows):

| regions touched | share of single-crate PRs |
| --- | --- |
| `src` only | 19.9% |
| `docs`+`src`+`tests` | 15.3% |
| `docs`+`manifest`+`src`+`tests` | 11.0% |
| `src`+`tests` | 10.8% |
| `tests` only | 10.6% |
| `docs`+`src` | 6.3% |
| `docs`+`manifest`+`src` | 4.5% |

The maintainer's hypothesis had a stated weak point — "add a failing test then make it pass" — and
the data confirms it: **42.5% of single-crate PRs move `src` and `tests` together**, and the crate
`README.md` and `Cargo.toml` ride along often enough that `docs` and `manifest` are not independent
regions either (26.8% of single-crate PRs touch crate docs alongside `src`). Only 19.9% are the
clean `src`-only case that region partitioning is designed for.

### How much parallelism would region partitioning actually unlock?

Concurrency proxy: pairs of PRs merged within a 24 h window (6 h and 168 h reported for
sensitivity). For pairs sharing at least one real crate, I ask two questions — do they share a file
*inside the shared crate* (would they textually conflict?), and are they `(crate, region)`-disjoint
inside it (would region partitioning have admitted both?).

| window | pairs | share a crate | share a file in that crate | `(crate,region)`-disjoint |
| --- | --- | --- | --- | --- |
| 6 h | 56 194 | 1637 | 1026 (62.7%) | 181 (**11.1%**) |
| 24 h | 187 653 | 4814 | 2747 (**57.1%**) | 700 (**14.5%**) |
| 168 h | 815 313 | 16 698 | 8963 (53.7%) | 2491 (14.9%) |

**Region partitioning inside crates has a ~14.5% ceiling.** Per hot crate it is worse where it
matters most:

| crate | co-24h pairs | `(crate,region)`-disjoint | share a file in-crate |
| --- | --- | --- | --- |
| `sparq-engine` | 790 | 18.2% | 48.0% |
| `sparq-server` | 757 | **10.2%** | **70.7%** |
| `sparq-conformance` | 344 | **3.8%** | 68.6% |
| `sparq-core` | 368 | 9.0% | 55.2% |
| `sparq-vectors` | 249 | 20.5% | 57.0% |

`sparq-server` and `sparq-conformance` — the second and third busiest crates — would gain almost
nothing. **This measurement kills strategy 1 as applied to crates**, and that is a valid outcome.

### The same-crate conflict rate (what the lock is preventing)

**57.1% of same-crate 24 h pairs share a file inside the shared crate.** Restricting to pairs
where both PRs touch `src/` (3278 pairs), **63.9% share an `src` file**. The lock is not paying for
a rare event; on the crates the fleet actually works, more than half of concurrent same-crate work
would collide textually — before considering semantic collisions, which no textual test sees.

---

## 3. The finding that redirects the design: the pressure is outside the crates

Same measurement, per partition, counting co-24h pairs *within* each partition and the fraction
that share a file within it. A partition with **many pairs and a low overlap rate** is
over-serialising — it is blocking work that would not have conflicted.

| partition | merged PRs | co-24h pairs | share a file | rate |
| --- | --- | --- | --- | --- |
| `skills/` | 679 | 19 147 | 2043 | 10.7% |
| `.github/` | 472 | 7716 | 1189 | 15.4% |
| repo root | 288 | 5901 | 504 | 8.5% |
| `scripts/` | 391 | 5181 | 642 | 12.4% |
| **`research/`** | 340 | 4909 | 107 | **2.2%** |
| `bench/` | 306 | 4312 | 536 | 12.4% |
| `__workspace__` | 302 | 3714 | 1790 | **48.2%** |
| `site/` | 185 | 1485 | 233 | 15.7% |
| `sparq-engine` | 138 | 790 | 379 | 48.0% |
| `sparq-server` | 107 | 757 | 535 | 70.7% |
| `sparq-core` | 80 | 368 | 203 | 55.2% |

Aggregated over all partitions with ≥15 merged PRs:

| group | co-24h pairs | file-overlapping |
| --- | --- | --- |
| real-crate partitions | 4943 | 2787 (**56.4%**) |
| non-crate partitions | 53 009 | 7236 (**13.7%**) |

The non-crate buckets carry an order of magnitude more serialisation pressure at roughly a quarter
of the collision rate — and §1 confirms it end-to-end: **58.3% of live deferrals are held by a
non-crate area.** Supporting file-level structure:

- `research/` is effectively **append-only**: 340 PRs across 248 distinct files, 58.5% of files
  touched by exactly one PR ever, 2.2% pair overlap. Serialising it is close to pure waste.
- `bench/` data: 461 distinct files, **81.3% touched by exactly one PR**.
- `skills/` is different: only **38** distinct files, and the hot ones are touched 88 / 74 / 62 / 52
  times. Its low pair-overlap rate comes from breadth across 38 files, not from append-only-ness.
- `.github/` is genuinely hot at the file level (`ci.yml` touched 108 times) — it should stay
  coarse, or be split only with the parent fallback of §7.
- `__workspace__` (`Cargo.lock`, workspace manifest) is **48.2%** overlapping. It must stay coarse.
  7.8% of all merged PRs touch `Cargo.lock`; that is the global collision the maintainer flagged,
  and the existing memory note *"same-new-dep beads collide on cargo-vet"* already documents it.

---

## 4. The three strategies, judged against the measurements

The governing constraint is stated once and applies to all three: **under-serialisation is the
corrupting direction.** A wrong narrow reservation puts two workers on one file — a merge conflict
at best, and at worst two diffs that each compile and pass and are together broken, which git
cannot detect. A wrong broad reservation only costs delay. Every option below is judged on what
happens when its prediction is wrong.

### Strategy 1 — region partition (`crate:region` leases): REJECT for crates, ADOPT for non-crate buckets

- **Inside crates: rejected.** 14.5% unlock ceiling (10.2% on `sparq-server`, 3.8% on
  `sparq-conformance`); 42.5% of single-crate PRs move `src`+`tests` together; `docs` and
  `manifest` are not independent of `src`. The shared-file hazards the maintainer named are real
  but secondary — a crate `Cargo.toml` is touched by 15.6% of merged PRs, and `[[bench]]` stanza
  additions are a rounding error here (only 5 merged PRs touched `benches/` at all, so
  bench-vs-dep manifest contention is a hypothetical, not an observed cost).
- **Failure mode if wrong:** benign *iff* the region map is a true file-system partition and the
  reservation is derived from the same map as the check. It fails toward over-reservation only if
  an unclassifiable file maps to the parent. That property is what §7 keeps.
- **Applied to the non-crate buckets: worth building.** 58.3% of live deferrals, 13.7% collision
  rate, and `research/` at 2.2% is nearly free parallelism.

### Strategy 2 — optimistic concurrency (admit, detect at merge, retry the loser): REJECT

Both sides quantified, as asked.

- **Collision side:** 57.1% of same-crate 24 h pairs share a file; 63.9% for `src`-vs-`src`. So
  more than half of admitted same-crate pairs lose the race.
- **Retry-cost side:** I sampled 98 completed `merge_group` workflow runs — per-run duration p50
  ≈ 1.6 min, p90 ≈ 19.9 min, max ≈ 24.9 min. End-to-end merge-group latency is the maximum across
  the group's workflows, so ~20 min is the right figure, corroborating the maintainer's stated ~18
  min median. **These timings are non-canonical** (GitHub-hosted runner variance, single sample
  window). All 98 sampled runs succeeded, so I have **no empirical bisect-frequency data** — the
  bisect cost in `batch-merge.py` (documented there as ~log2(15) ≈ 4 runs for a 15-wide batch) is
  designed-for, not observed in this window. I will not present it as measured.
- **The decisive argument is architectural, not arithmetic.** "Retry the loser" presumes a live
  agent that can rebase. This fleet is fire-and-forget: the loser is a Haiku/Sonnet worker whose
  session has already ended. Retrying means **re-dispatching the whole bead** — full worker cost
  and full CI cost, not a cheap rebase — and on a semantic (non-textual) conflict there is nothing
  for a rebase to catch at all. Expected cost per admitted same-crate pair is therefore
  ≈ 0.571 × (whole re-implementation + full gate), against a serialisation cost of one queue-slot
  wait. That is not close.
- **Is the merge-group gate a sufficient backstop for semantic conflicts?** No. It is a real
  backstop and it would catch the compile-and-test-visible cases, but (a) a failed group dequeues
  and costs a bisect over the whole batch, penalising the innocent members, and (b) it only catches
  what the test suite covers — and this repo has measured, repeatedly, that fleet-authored tests
  are frequently vacuous (`fleet output fails cross-provider review`: 10/10 failed cross-provider
  review with green CI). Relying on the gate to catch semantic conflicts leans on exactly the
  evidence source this project has already measured as unreliable.

### Strategy 3 — predicted-file locking: REJECT

- **Ceiling:** 42.9% of same-crate 24 h pairs are file-disjoint inside the shared crate — genuinely
  3× region partitioning's 14.5%. The upside is real.
- **But it cannot bound its own error rate.** The prediction input is issue prose. The existing
  precedent is `bd-to-issues.py`'s `derive_areas`, which infers `area:` from bead text and whose own
  in-tree comment records a live misfire (*"the whole title mislabeled a zkSPARQL spec bead
  `area:site` off one incidental word"*). Nothing in the pipeline validates a predicted file set
  against the PR that eventually lands, so there is no feedback signal from which an error rate
  could even be estimated, let alone bounded.
- **And the base rate makes a wrong-narrow prediction the likely case, not the tail case.** Given
  two workers already admitted into one crate, 57.1% of historical pairs shared a file. A
  prediction that misses one file therefore lands on a genuine collision more often than not.
- Per the governing constraint, **a file-prediction scheme that cannot bound its own error rate
  should be rejected**, and this one cannot. Rejected. Revisit only if §8's Phase 2 produces a
  measured prediction-accuracy record from shadow-mode data.

---

## 5. Blocking defect found while measuring: sub-crate `area:` labels already break the lock

The repository has **98 `area:` labels**: 62 exactly matching a crate directory, 31 non-crate, and
**5 that name a region *inside* an existing crate**:

| label | files live under | parent label |
| --- | --- | --- |
| `area:sparq-server-http` | `crates/sparq-server/` | `area:sparq-server` |
| `area:sparq-core-nt-dict` | `crates/sparq-core/` | `area:sparq-core` |
| `area:sparq-core-store` | `crates/sparq-core/` | `area:sparq-core` |
| `area:sparq-engine-exec` | `crates/sparq-engine/` | `area:sparq-engine` |
| `area:sparq-conformance-floors` | `crates/sparq-conformance/` | `area:sparq-conformance` |

`compute_ready()`'s `conflict()` compares partition keys by **set overlap on exact strings**. A
sub-crate key therefore does not overlap its parent. Demonstrated directly against the live
function:

- an issue labelled `area:sparq-server` and an issue labelled `area:sparq-server-http`
  **both enter the frontier in the same tick**;
- an issue labelled `area:sparq-server-http` **enters the frontier despite an open PR holding
  `area:sparq-server`**.

Live exposure in the current open backlog: 26 issues on `area:sparq-server` vs 3 on
`area:sparq-server-http`; 24 on `area:sparq-core` vs 2 + 2 on `area:sparq-core-nt-dict` /
`area:sparq-core-store`; 47 on `area:sparq-engine` vs 2 on `area:sparq-engine-exec`; 9 on
`area:sparq-conformance`. Two of the ten rows on the frontier I sampled were sub-crate keys
(`#3186` `sparq-server-http`, `#3188` `sparq-core-nt-dict`).

**This is the maintainer's idea already half-implemented, in the corrupting direction.** Someone
introduced finer keys to get more parallelism without teaching the conflict predicate that the
finer key is *contained in* the coarser one. It is a P1 correctness bug, it is independent of every
optimisation in this record, and it must be fixed first. Filed as a GitHub issue (see §10) rather
than as an implementation bead, per the brief.

Note this also means the 14.5% region-partition ceiling in §2 is not merely a modelling exercise —
five hand-made region keys already exist in production, and they bought a frontier of 10.

---

## 6. Interaction with what exists — the fail-closed default must survive

Constraints any finer scheme must preserve:

1. **`packages_of` (candidate side) keeps its empty → `__global__` fallback.** A candidate we
   cannot attribute serialises against everything. Cost of being wrong: one dispatch.
2. **`_reserving_packages` (occupancy side) keeps its *lack* of a global fallback.** The in-tree
   `[OPUS-5]` comment records why: an unattributable *occupant* that seized every crate reduced the
   whole frontier to zero, because nothing in the pipeline applies `area:` labels to PRs. That
   asymmetry is deliberate and load-bearing — do not "fix" it symmetrically.
3. **The registry's `busy_packages_of_pulls`** (in the private `agent-account-registry`
   `dispatch.yml`, not in this repo) unions a PR's `area:` labels with its provenance-linked source
   issue's, and a PR with no provenance record fail-closes to `__global__`. That default must not be
   weakened. Per the maintainer's brief this hole has stalled the fleet six times today; a finer key
   space multiplies the number of ways an unrecognised key can slip past, so **every new key must
   have a total, defaulting mapping to a parent** — an unknown or unclassifiable key resolves to the
   parent partition, never to "no conflict".
4. **Parity is mandatory.** The partition key is derived in *two* places — `ready-issues.py` here
   and the registry's `dispatch.yml` — and they must agree exactly or the fleet double-dispatches.
   Any change ships with a parity fixture asserted on both sides. `dispatch-plan.py` already carries
   `#3691` fixtures that exist because the two disagreed on multi-area issues and perma-deferred
   them; that is the precedent.

---

## 7. Which layer to change, and why

The brief notes that fixing at the wrong layer is this codebase's most common defect today, so this
is stated explicitly.

**Change the partition-key algebra — `conflict()` plus the key-derivation helpers
(`packages_of` / `_reserving_packages`) — not `compute_ready()`'s selection loop, and not the
label vocabulary alone.**

- **Not the label vocabulary.** Adding or renaming labels is what produced the §5 defect: new keys
  with unchanged comparison semantics. The vocabulary is the *input*; the bug is in the comparison.
- **Not `compute_ready()`'s loop.** The loop is a correct priority-ordered greedy reservation over
  an abstract key set. It has no business knowing that `sparq-server-http` sits inside
  `sparq-server`. Teaching it would duplicate that knowledge into the registry's copy too.
- **Yes `conflict()` + key derivation.** Replace exact-string set overlap with a **containment-aware
  overlap** over a hierarchical key (`crate` → `crate/region`, `surface` → `surface/subpath`), where
  the mapping from key to partition path is **total**: an unrecognised key resolves to its longest
  recognised ancestor, and ultimately to `__global__`. Then:
  - `sparq-server-http` resolves under `sparq-server` and correctly conflicts with it (§5 fixed);
  - two keys conflict iff one is a prefix of the other, so a wrong or unknown key over-reserves
    rather than under-reserves — **the failure direction is structurally correct, not
    convention-dependent**;
  - the same predicate serves crate regions and non-crate subpaths, so §3's win and §5's fix are
    one mechanism, not two;
  - it is a pure function, unit-testable in `--self-test`, and mirrorable into the registry with a
    parity fixture per §6.4.

The lease/`partition_available` layer is the right *scope*; `compute_ready` is the wrong *site*.

---

## 8. Staged rollout, with the measurement gate

**This is not the currently binding constraint.** Worker yield is (~20–86% depending on model, per
the maintainer's brief). This project has measured two of three plausible optimisations as
worthless — `ast-grep`/outline-first was a token *increase*, `sparq-terse` saved ~0% — and one of
the three strategies here is being rejected outright with a third partly rejected. Assume the same
prior and do not build past a gate that has not opened.

- **Phase 0 — correctness, unconditional, do now.** Containment-aware `conflict()` (§7), so the five
  existing sub-crate labels stop under-serialising. No new keys introduced. Acceptance: a
  `--self-test` fixture where `area:sparq-server` and `area:sparq-server-http` conflict in both
  directions and an unknown key `area:sparq-server-zzz` conflicts with `area:sparq-server`; plus the
  registry parity fixture. This is a bug fix and is not gated on anything below.
- **Gate A — is the lock binding yet?** Instrument the dispatcher to record, per tick,
  `frontier_width`, `realised_dispatches`, and `conflict_deferrals` attributed by held area. Open
  the gate only when realised dispatches ≥ ~80% of frontier width for a sustained period — i.e.
  when worker yield has stopped being the ceiling and the frontier is genuinely the limit. Until
  then, widening the frontier adds plan rows nobody executes.
- **Phase 1 — split the non-crate buckets only, behind Gate A.** Declared file-glob subpartitions
  for the buckets §1 and §3 identify: `bench`, `ci`, `gui`, `site`, `docs`/`research`, `upstream`.
  Each subpartition is a path prefix under the parent, and anything not matching a declared glob
  resolves to the parent (over-reserve on miss). Expected reach: the 211 non-crate deferrals, at a
  13.7% historical collision rate, with `research/` at 2.2%. Measure the delta in realised
  dispatches; if it does not move, stop here and record that.
- **Phase 2 — shadow-mode file prediction, measurement only, no leasing.** If and only if Phase 1
  paid, record for each dispatched issue the file set a predictor *would* have reserved, then
  compare it with the file set the merged PR actually touched. That produces the
  prediction-error-rate record strategy 3 currently lacks. **No lease is ever narrowed on a
  prediction until that record exists and bounds the error.** If the record cannot be produced,
  strategy 3 stays rejected permanently.
- **Never — region partitioning inside crates as a parallelism lever.** 14.5% ceiling, 3.8% on
  `sparq-conformance`. The containment fix in Phase 0 makes the *existing* sub-crate keys safe; it
  is not an invitation to mint more of them.

---

## 9. Why there are no implementation beads

Deliberate, per the maintainer's brief: the separability measurement determines which strategy is
worth building, and two of the three are being rejected on that measurement. Beads would presuppose
a decision the maintainer should make with §2–§5 in hand. The one item that is *not* a design
question — the §5 under-serialisation defect — is filed as a GitHub issue so it can be scheduled
without waiting on this record.

## 10. Provenance and honesty notes

- Sample: 2167 merged PRs, full PR-carrying history of `origin/main` (~43 days), from
  `git log --name-only` on squash-merge subjects matching `(#N)`. Paginated-API caps were avoided by
  using local git rather than `gh pr list`.
- Live frontier figures are a single snapshot taken 2026-07-26 and will drift.
- Concurrency is proxied by **co-merge within a time window**, not by observed
  simultaneously-open PRs. It is an approximation: it over-counts pairs that were never in flight
  together and under-counts long-lived PRs. It is a *file-structure* measurement, which is what the
  region question needs; it is not a measurement of realised conflicts.
- File-set overlap is a **necessary** condition for a textual conflict, not a sufficient one, and
  says nothing about semantic conflicts. The 57.1% figure is therefore an upper bound on textual
  conflicts and a lower bound on total conflict risk.
- `merge_group` durations in §4 are a single-window sample on GitHub-hosted runners and are
  **non-canonical**. No bisect frequency was observed; none is claimed.
- No cryptographic claim is made anywhere in this record. `area:zk` / `area:sparq-mpc` /
  `area:zk-xpath` appear only as scheduler label strings. The external accredited-cryptographer
  audit gate (`sq-qhy4`) is untouched by anything here, and no scheduling change in this record may
  be read as bearing on ZK or MPC soundness — the v1 verifier remains internally re-audited with
  external sign-off pending, and MPC remains semi-honest-only.

### Cross-references

- `scripts/ready-issues.py` — `compute_ready`, `conflict`, `packages_of`, `_reserving_packages`.
- `scripts/dispatch-plan.py` — `plan_dispatch`, and the `#3691` package-semantics parity fixtures.
- `scripts/bd-to-issues.py` — `derive_areas` (the prose-to-`area:` inference), `_SINGLE_VALUED`.
- `scripts/batch-merge.py` — the v2 culprit bisect that prices a failed merge group.
- `research/issue-native-orchestration-review-gpt56.md` — the review that produced the fail-closed
  readiness posture this record must not weaken.
