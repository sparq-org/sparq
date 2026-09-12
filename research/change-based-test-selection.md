# Change-based test + benchmark selection for CI (sq-fmx4u)

**Status:** design record (provisional — graduates into the AGENTS.md gate table +
`.github/` docs once implemented). Authored under the proceed-and-document rule.
**Author:** Claude Fable 5 (SPARQ architect tier), 2026-07-02. [FABLE-5]
**Implementation:** decomposed into disjoint child beads under `sq-fmx4u` (§8).

---

## 1. Problem

Every PR — and every merge-queue entry, since each queue entry re-runs its own
checks — executes the full required matrix: per-crate `cargo test`, the ~50
per-crate opt-in feature legs (`feature-matrix.yml`), benchmark jobs, fuzz
smoke, the coverage ratchet, wasm builds, CodeQL, and the docs gates — on the
order of ninety required checks. Runner job-slots are the binding resource.
Most PRs touch exactly one crate (a coverage test in `sparq-geo`, a leaf
feature in `sparq-rsp`), yet pay the whole matrix. Throughput is therefore
bounded by *matrix width × queue depth*, not by verification need.

**Goal.** For each PR, run a package's test + benchmark jobs only when that
package **or its transitive dependency closure** changed in the diff; skip the
rest with a green `skipped` conclusion that still satisfies branch protection,
the `ci-summary` aggregator, and the merge queue. A wrong skip is a *silent*
correctness hole (a red test simply never runs), so the design is governed by
an explicit fail-safe invariant (§2), a fail-closed mechanism (§4.3), and a
full-run backstop (§6).

**Expected effect** (estimate, not a measurement — the shadow-mode rollout in
§6.4 produces the real numbers): a leaf- or near-leaf-crate PR shrinks from
the full matrix to roughly its own closure's jobs plus the always-run lanes,
i.e. an order-of-magnitude reduction in runner-minutes for the common case,
and correspondingly shorter merge-queue occupancy per entry.

## 2. The skip invariant (normative)

> **A crate C's jobs may be skipped for a revision pair (base, head) only if
> every path changed between base and head is either (a) owned by a package
> outside C's transitive dependency closure, or (b) on the audited SAFE list
> of paths that no build, test, benchmark, or CI definition reads. Any changed
> path that is neither — and any failure to compute the above — forces the
> FULL matrix.**

Equivalently: *a test is skipped ⇒ provably no change could affect it.*
Skipping is an optimization applied to a proof of non-interference; absence of
proof means run. Every rule below is an instantiation of this invariant, and
every implementation choice defaults to running more, never less.

## 3. Affected-set algorithm

Computed once per workflow run by a cheap pre-job (`select`, §5.1) running a
small selector script (§7 position P1).

### 3.1 Changed paths

- `pull_request`: `git diff --name-only --no-renames <base-sha>...<head-sha>`
  (three-dot = against the merge-base with the target branch). `--no-renames`
  reports a rename as delete + add so **both** paths are attributed —
  conservative for moves between crates.
- `merge_group`: same diff against the target branch tip. A queue entry's diff
  therefore contains the union of every queued change ahead of it — exactly
  the conservative set, so selection stays sound inside the queue while still
  saving width (§7 P8).
- `schedule` / `workflow_dispatch` / the `ci-full` label: no diff — `mode=full`
  (§6).
- If the base SHA cannot be fetched (shallow-clone trouble, force-push race):
  `mode=full` (§4.3).

### 3.2 Path → package ownership

From `cargo metadata --format-version 1 --locked`:

- Build the set of **in-repo path packages** — all workspace members *plus*
  any non-member path dependencies that live inside the repository (e.g. a
  vendored path dep). Each package owns the directory of its `Cargo.toml`.
- A changed file is owned by the package with the **longest matching directory
  prefix** (handles nested crates).
- Files owned by no package are looked up in the explicit ownership map
  `ci/path-ownership.toml` (§4.2): an entry attributes them to listed crates,
  or marks them SAFE. **An unmapped, unowned path forces `mode=full`.**

Granularity is deliberately whole-crate: any change under a crate's directory
marks the crate changed — no "only `benches/` changed" refinement (§7 P4).
This includes the crate's `Cargo.toml`, `build.rs`, `README.md` (routinely
pulled into doctests via `#[doc = include_str!(...)]`), tests, benches, and
fixtures under the crate dir.

### 3.3 Reverse-dependency closure

- Dependency edges are taken from the **package-level** dependency lists in
  `cargo metadata` (`packages[].dependencies` filtered to in-repo path
  packages), including **normal, dev, and build** kinds, **optional**
  dependencies regardless of feature activation, and **target-specific**
  dependencies (e.g. `cfg(target_arch = "wasm32")`) regardless of host. This
  is a superset of any feature-resolved graph, hence conservative (§7 P2).
  Dev-dependencies matter: if A is a dev-dep of B, changing A can break B's
  tests.
- `affected = ⋃ over changed crates c of ({c} ∪ reverse-closure(c))`,
  intersected with the workspace-member set for job selection.
- Feature-unification note: CI test jobs build per-crate (`cargo test -p C` /
  per-leg `-p C --features ...`), so a feature or dependency declaration
  change in member manifest `A/Cargo.toml` can only influence builds whose
  graph contains A — exactly A's reverse closure. (`[patch]` and `[profile]`
  are only honored in the workspace **root** manifest, which is a full-run
  trigger.) Any member-manifest change that alters resolved versions also
  rewrites `Cargo.lock` → full run anyway.

### 3.4 Edge cases (all resolve to "run more")

| Case | Resolution |
|---|---|
| New crate added | root `Cargo.toml` members change → full run |
| Crate removed / renamed | root `Cargo.toml` changes → full run |
| File moved between crates | `--no-renames` marks both crates changed |
| Deleted file inside a surviving crate | still prefix-owned by that crate |
| Symlink whose target crosses crate dirs | audit item (§4.2); unowned → full |
| `build.rs` or test reading outside its crate dir (`../`, `include_str!` escaping the crate) | must be enumerated in the ownership map by the audit (bead 2); unmapped shared dirs are unowned → full |
| Selector cannot parse metadata / diff / map | `mode=full` (§4.3) |

## 4. Fail-safe rules

### 4.1 Unconditional full-run triggers

Any changed path matching the following forces `mode=full`, no closure math:

| Trigger | Why a skip would be unsound |
|---|---|
| `Cargo.lock` | resolved versions feed every build |
| root `Cargo.toml` | members, `[workspace.dependencies]`, lints, profiles, `[patch]` |
| `rust-toolchain`, `rust-toolchain.toml` | compiler version affects all codegen |
| `.cargo/**` | rustflags / registries / build config are global |
| `.github/**` | the CI definition itself, including the selection wiring |
| `scripts/**` | shared gate/coverage/check scripts executed by CI |
| `deny.toml`, supply-chain / vet / audit / SBOM config | changes what "green" means for the supply-chain lanes |
| `ci/path-ownership.toml` | the selection policy itself |
| any path owned by no package and not SAFE-listed | unattributable ⇒ unprovable ⇒ full (§2) |

Changes to the selector script or its tests are subsumed by `scripts/**`; a
selection-logic PR therefore always validates against the full matrix.

### 4.2 The ownership map — `ci/path-ownership.toml`

A small, checked-in, audited policy file. Three ownership-verdict forms plus the
monotone `readers` union (below):

```toml
# Attribute a non-crate path to the crates whose tests read it:
[[map]]
pattern = "testsuites/w3c-sparql/**"
crates = ["sparq-conformance"]

# Prove a path inert for the Rust matrix (docs lanes still run):
[[map]]
pattern = "research/**"
safe = true

# Additional-readers: union extra reader crates for a CRATE-OWNED path where a
# real dep edge would be a cycle (monotone; see below):
[[map]]
pattern = "crates/sparq-solid/rules/**"
readers = ["sparq-reason"]

# Everything unmapped and unowned => mode=full (implicit default).
```

Rules:

- **The SAFE list starts empty.** Entries are added only by the audit (bead
  2), which greps for out-of-crate inputs — `include_str!`/`include_bytes!`
  escaping a crate dir, `../` path literals in tests/build scripts, repo-level
  files ingested by tests (e.g. `sparq-kb` PKG tests read `AGENTS.md` and
  `skills/**` — those paths must map to `sparq-kb`, not SAFE). Candidate SAFE
  entries to evaluate, not presume: `research/**`, `site/**` (owns its own CI
  lane), `.beads/**`.
- A map-validity unit test asserts every `crates` name is a current workspace
  member and every literal pattern root exists — the map cannot silently rot
  when crates move.

**Additional-readers (monotone union)** — sq-m4bxc [FABLE-5]. A fourth verdict
form, `readers = [...]`, added as a *deliberate deviation from pure
input-relocation*. The two closed residuals were sibling reads across a boundary
where the "attribute the shared input to both crates" fix is not available: the
input lives *inside* a crate dir (so crate-prefix ownership, steps 1–2, wins
before the map is ever consulted for a `crates` attribution) **and** the reading
crate is not in the owner's reverse-dependency closure. Relocation was rejected
because it would move a *vendored crypto ontology*
(`crates/sparq-secprop-vocab/ontologies/secprop-ext.ttl`; it lived under
`crates/sparq-trust/ontologies/zkp-sparql/` until #3705 moved it to a leaf crate) and a *crate's core
authorization rule corpus* (`crates/sparq-solid/rules/*.n3`) out of their owning
crates and rewrite production `include_str!` / runtime include paths in
`sparq-trust`, `sparq-solid`, `sparq-zk` and `sparq-reason`. The dep-edge
alternative is a **cargo cycle** in both cases (`sparq-trust` depends on
`sparq-zk`; `sparq-solid` depends on `sparq-reason`), so `reason -> solid` /
`zk -> trust` edges are impossible.

`readers` is the one verdict form consulted **even for a crate-owned path**: it
UNIONS the listed crates into the changed-crate set *in addition to* the path's
normal prefix owner, without changing the ownership verdict. This makes it
strictly **monotone / fail-safe** — adding a `readers` entry can only ENLARGE the
affected set (it never rescues an unowned/unmapped path from `mode=full`), so it
cannot introduce the unsound skip §2 forbids; a unit test pins that monotonicity
property. The out-of-crate-input audit (bead 2) treats a sibling read as covered
iff the reader is listed in a matching `readers` entry, and the map-validity test
extends to `readers` names. Residual 3 (`sparq-conformance`'s
`scoreboard_floors.rs` reading sibling **test sources** at a statically
unresolvable runtime path) was *not* coverable by `readers` — the union matches a
literal/glob pattern, and a path assembled at runtime collapses to the workspace
root — so it stayed acknowledged, backstopped by the nightly full run (§6.1),
pending its own design pass.

**Residual 3 closed by input-relocation** — sq-z1xv8 [SONNET-4.6]. The read
existed only because a floor enforced in one crate's runner had to reach
`sparq-conformance`'s central `scoreboard::SUITES` without a dep edge, so the
number was spelled twice and reconciled by reading the other crate's test source.
The eleven such floors (W3C SHACL core + SHACL-SPARQL, both OGC GeoSPARQL lanes,
Solid WAC/ACP decision parity + the two differential oracles, the SolidLab ODRL
suite, the sparq-text BM25 oracle, the sparq-rsp expressivity oracle) moved into a
new leaf crate, `sparq-conformance-floors`: zero dependencies, `publish = false`,
constants only. Each enforcing crate takes it as a **dev-dependency** (shipping
graph untouched) and `sparq-conformance` as a plain dependency, so both sides read
ONE compile-time `const` — the same cannot-drift shape the six JSON-LD lanes
already had via `sparq_conformance::floors` — and the guard reads no foreign source
at all. This is genuine relocation, not a coverage waiver: the dep edges put every
enforcing crate in the floors crate's reverse-dependency closure, so a floor change
cannot selection-skip the lane that enforces it. The surviving textual reads are
rooted at `CARGO_MANIFEST_DIR` and a unit test
(`textual_guard_reads_only_crate_local_sources`) fails if a row ever names a path
outside the crate, so the hole cannot silently reopen. `KNOWN_RESIDUALS` in
`scripts/ci_audit_inputs.py` is now **empty** — every out-of-crate input in the
workspace is covered by a trigger, a map attribution, a dep closure or a `readers`
union.

### 4.3 Fail-closed mechanics

- The selector traps **all** internal errors and emits `mode=full` with exit
  0 — selector bugs degrade to the status quo, never to a skip.
- Downstream job guards are written so an **empty/missing output means run**:
  the run-condition is `mode != 'selected' || crate ∈ affected`, and an unset
  `mode` (select job failed, output lost) satisfies the first disjunct.
- If the `select` job itself hard-fails, `ci-summary` goes **red** (§5.3) —
  an infrastructure failure blocks merge rather than silently running full or
  silently skipping.

## 5. CI wiring

### 5.1 The `select` pre-job

One cheap job (<1 min): checkout with enough history to resolve the
merge-base, run `scripts/ci_select.py` (stdlib-only Python — no compile step,
`cargo metadata` is already JSON; precedent: `coverage-gate.py`,
`check-readme-template.py`; §7 P1). Outputs:

- `mode` ∈ {`selected`, `full`} + a human `reason`
- `affected` — JSON array of workspace-member names (when `selected`)
- a step-summary table: changed files → owning crates → closure → skipped
  count, so every PR shows its selection reasoning to reviewers.

### 5.2 Consumption — which lanes are scoped

**Phase 1 scopes only the wide multipliers** (this is where the ~90-check
width lives); the singleton lanes stay always-run so the initial soundness
surface is minimal (§7 P7):

| Lane | Phase 1 | Mechanism |
|---|---|---|
| per-crate test jobs | **scoped** | job-level `if:` guard (below) |
| `feature-matrix.yml` ~50 opt-in legs | **scoped** | same guard, keyed on the leg's `-p` crate |
| benchmark jobs | **scoped** | same guard |
| workspace nextest archive / bulk partitions | **scoped by filterset** | if a job runs a cross-crate partition, don't skip the job — narrow it: `cargo nextest run -E 'package(a) + package(b) ...'` over the affected set. The archive/compile job runs whenever `affected ≠ ∅` |
| lint / fmt / clippy, docs-quality, readme-template | always-run (cheap) | — |
| coverage ratchet | always-run (single job; exception: skip only when `affected = ∅`, since coverage is a function of compiled code + tests) | — |
| CodeQL, supply-chain (deny/vet/SBOM) | always-run (security semantics are whole-repo) | — |
| fuzz smoke, wasm builds, perf-gate | always-run in phase 1; **phase 2** scopes them by mapping each to the crate closure it exercises (bead 6) | — |
| `select` itself, `ci-summary` | always-run | — |

The wiring bead reconciles this table against the real job topology in
`ci.yml` / `feature-matrix.yml` / the bench workflows — the design fixes the
*semantics* (guard expression, filterset narrowing, fail-closed defaults);
the implementation maps them onto the actual jobs.

**The guard** (job-level `if:` on a static matrix — §7 P5): a job skipped by
a job-level `if:` is decided server-side (no runner slot consumed), keeps its
check name, and reports conclusion `skipped`, which GitHub treats as
satisfying a required status check. Implementation traps to honor:

- Do **not** `fromJSON` a possibly-empty output inside `if:` (expression
  error ⇒ workflow failure). Use delimited string containment instead:
  `contains(needs.select.outputs.affected, format('"{0}"', matrix.crate))` —
  the quote delimiters prevent `sparq-core` matching inside
  `sparq-core-foo`.
- Full run-condition:
  `needs.select.outputs.mode != 'selected' || contains(...)` — empty output
  ⇒ run (§4.3).

> **⚠️ ERRATUM (bead sq-fmx4u.7, 2026-07-05, PR #1437). [HAIKU-4.5]**
>
> **The prescriptive design above is NOT implementable as written for the
> feature-matrix legs.** The design (and §7 P5) specify a *job-level* `if:`
> guard keyed on `matrix.crate` — e.g.
> `contains(needs.select.outputs.affected, format('"{0}"', matrix.crate))`
> evaluated in `jobs.<job_id>.if`. GitHub Actions does **not** make the
> `matrix` context available in `jobs.<job_id>.if`: per the Actions
> *contexts-availability* table, a job-level `if:` can reference only
> `github`, `needs`, `vars`, and `inputs` — **not `matrix`**. A `matrix.crate`
> reference there silently evaluates to the empty string, so under enforcement
> the guard degrades to `contains(affected, '""')`, which is always false and
> would skip **every** leg unconditionally — precisely the silent-unsound-skip
> class the skip invariant (§2) forbids. This is an infeasibility of the
> original design, **not** an optimization trade-off: the job-level
> `matrix.crate` guard cannot be built on GitHub Actions at all.
>
> **What shipped instead** (both fail-closed, both pinned by
> `scripts/tests/test_ci_select_wiring.py`, which additionally asserts that no
> job-level `if:` ever references `matrix.`):
>
> 1. **Feature-matrix legs → assembly-time leg filtering** in the unit-tested
>    `scripts/assemble-feature-matrix.py` (see
>    `.github/workflows/feature-matrix.yml`, the `select`/`setup` jobs). The
>    per-leg keep/drop decision is made in Python before the matrix is
>    materialized; an unassembled leg spawns **no** check-run, and the polling
>    `ci-summary` aggregator never waits on a check that does not exist
>    (requiredness flows through `ci-summary / gate`, not per-leg names —
>    verified against live branch protection by bead sq-fmx4u.4). Shadow /
>    full / missing selection outputs fail-close to the full leg set inside the
>    assembler.
> 2. **Heavy `crates/sparq-vectors` shards → a step-level bash guard** in the
>    test step (see `.github/workflows/ci.yml`, the shard test step). The *step*
>    context — unlike the job-level `if:` — **does** see `matrix`, so the guard
>    reads `${{ matrix.crate }}` and `exit 0`s with a skip notice (reporting
>    `success`, not conclusion `skipped`) when the shard's crate is absent from
>    the affected closure. Fail-closed: only `mode = 'selected'` with a
>    non-empty crate and a definite non-membership can skip; empty/missing
>    selection outputs fall through to a run.
>
> The prescriptive text and §7 P5 are retained above as the original decision
> record; this erratum is the correction. Where §5.2/§7 P5 say "job-level
> `if:` guard" for the feature-matrix legs, read "assembly-time filtering";
> the job-level `if:` guard remains accurate only for the per-crate lanes whose
> guard keys on `needs`-derived outputs, not on `matrix`.

### 5.3 `ci-summary` (the aggregator)

- `if: always()`; **fails** if the `select` job did not succeed; **fails** on
  any needed job with result `failure` or `cancelled`; treats `skipped` as
  satisfied **only when** `select` succeeded. Summary line reports
  "`selected` mode: N of M crate-jobs run, K skipped by selection".
- Branch protection / merge-queue requiredness continues to flow through
  `ci-summary` plus the individually-required checks. Because
  selection-skipped required checks report `skipped` (which branch protection
  counts as satisfied), the required-check *names* never go missing — this is
  why the static-matrix guard beats dynamic matrix generation, where an
  unspawned job leaves a required check "expected" forever and blocks merge.
**§5.3 graduation note (bead sq-fmx4u.4, verified 2026-07-03). [SONNET-4.6]**
Verified against the live ruleset (`gh api repos/sparq-org/sparq/rulesets/17688455`,
id 17688455, last updated 2026-07-02):

- `required_status_checks` contains **exactly one entry**:
  `{"context":"gate","integration_id":15368}` — `ci-summary / gate` is the
  sole required check. No individual per-crate matrix legs, no individual
  `opt-in <X>` legs appear in the required list.
- At the 2026-07-03 verification, `merge_queue` used
  `grouping_strategy: "ALLGREEN"` and `check_response_timeout_minutes: 60`;
  the queue blocked only on the required `gate` check, not on absent or skipped
  siblings. #6048 supersedes the grouping premise: `merge_group` is now forced
  to `mode=full`, which permits `HEADGREEN` to validate the complete combined
  head once without relying on selected-mode evidence from every prefix.
- A selection-skipped job (`conclusion=skipped`) from a static-matrix `if:`
  guard **reports a complete check-run** (the `skipped` conclusion is present,
  never "expected but missing"). `ci_summary_gate.py` already treats `skipped`
  as non-failing when select succeeded — so the gate passes, the queue
  unblocks, nothing hangs.
- Feature-matrix legs filtered at assembly time (unassembled — no check-run
  spawned) are also safe: since no leg name is individually required, the merge
  queue never waits for them; the only thing it blocks on is `gate`, which the
  aggregator always produces.
- **Mechanism chosen: plain-skip. No shim is needed.** The skipped-but-green
  shim (guard moved inside the job as a first step, so the job always occupies
  a slot but exits early) would be required only if a per-crate leg name
  appeared in `required_status_checks` — it does not. That condition is the
  only way plain-skip could cause a merge-queue hang.
- **What sq-fmx4u.5 can safely flip**: set the repo variable `CI_SELECT_MODE`
  to the literal string `"enforce"`. No ruleset change is required; no
  individual branch-protection entries need editing. The merge queue will not
  hang; skipped jobs will satisfy the gate through `ci-summary`.
- Three properties make this safe and must not drift; they are pinned by
  `scripts/tests/test_ci_select_wiring.py` (`TestRequiredCheckAnchor`):
  (1) the `ci-summary / gate` job name is exactly `"gate"` (matches the
  ruleset's `context:"gate"`); (2) that job has no `if:` guard (always scheduled,
  but completion before the deadline still depends on runner capacity and upstream
  checks; the live timeout was re-verified as 360 minutes on 2026-09-06);
  (3) `ci-summary.yml` triggers on `merge_group` (required for the
  gate to produce a check-run on queue entries at all). Under HEADGREEN a fourth
  property is load-bearing: (4) the reusable selector adds `--full` for every
  `merge_group` before either the label override or shadow escape hatch. The
  structural and mutation checks in `TestWiring` pin that seam.

## 6. Correctness safeguards

1. **Merge-group FULL run**: every `merge_group` event forces `mode=full`.
   HEADGREEN therefore gates the complete combined tree once; PR-head runs keep
   selected mode for fast feedback. Removing, inverting, or demoting the override
   fails the wiring mutation test.
2. **Nightly scheduled FULL run** on `main` (`schedule` event ⇒ `mode=full`).
   Any nightly failure whose job was selection-skipped in the PRs that landed
   since the last green nightly is *prima facie* a selection bug: auto-file a
   P1 bead/issue with the offending job + suspect PRs.
3. **`ci-full` label**: applying it forces `mode=full`; the workflow reacts to
   `labeled`/`unlabeled` so toggling re-evaluates. One auditable override, no
   commit-message magic.
4. **`workflow_dispatch`** manual full run for ad-hoc verification.
5. **Shadow-mode rollout**: a flag under which the selector computes and
   *reports* the would-skip set while every job still runs. Enforce only
   after a shadow window (order of twenty PRs) shows **zero** cases where a
   would-have-been-skipped job failed for a reason attributable to the PR.
   The shadow report is the honest measurement of both soundness and savings.
6. **Selector self-tests** (bead 1 + bead 2): golden diffs → expected sets —
   leaf change ⇒ exactly that crate; `sparq-core` change ⇒ all members;
   dev-dep and optional-dep edges propagate; every §4.1 trigger class ⇒ full;
   unowned path ⇒ full; internal error ⇒ full; plus a test pinned against the
   real workspace metadata so graph-shape regressions surface in review.
7. **Transparency**: the per-PR step-summary (§5.1) makes every skip decision
   reviewable where reviewers already look.

**§6 graduation note (bead sq-fmx4u.5, ENFORCEMENT FLIPPED 2026-07-03). [FABLE-5]**
The shadow rollout is now flipped to **enforce by default** — a not-affected
crate's wide-lane tests/benchmarks are actually skipped. This is the culmination
of the epic and is proven-safe by bead sq-fmx4u.4: the live ruleset requires
exactly one check (`ci-summary / gate`), so a selection-skipped leg reports
`skipped` (= satisfied) and can never individually hang the merge queue. No
ruleset change was needed. What landed:

- **Enforce flip** (`.github/workflows/ci-select.yml`): `--shadow` is now added
  *only* when the repo variable `CI_SELECT_MODE` is the literal `"shadow"` (the
  report-only rollback escape hatch); any other value — including unset and
  `"enforce"` — enforces. The pre-flip `!= "enforce" ⇒ --shadow` default is gone.
- **`ci-full` label override** (safeguard 2): the select step computes
  `CI_FULL_LABEL = contains(github.event.pull_request.labels.*.name, 'ci-full')`
  and, when true, runs the selector with `--full` (mode=full, nothing skipped).
  `ci.yml` + `feature-matrix.yml` now trigger on `pull_request:` types
  `[opened, synchronize, reopened, labeled, unlabeled]`, so toggling the label
  re-evaluates selection.
- **Nightly full-matrix backstop** (safeguard 1): the existing `schedule` cron on
  `ci.yml` resolves to `mode=full` by construction (a non-PR event carries no
  diff), and a new `selection-backstop` job asserts that `mode=full` invariant
  fail-loud on `schedule`/`workflow_dispatch` (it REDs if a scheduled run were
  ever narrowed). `workflow_dispatch` (safeguard 3) is the ad-hoc full run.
- **Fail-safe preserved** (non-negotiable): `ci_select.py` is unchanged — every
  §4.1 trigger (shared crate / build file / `.github/**` / `Cargo.lock` /
  selector-self change) and any internal error still return `mode=full`, so
  enforce never skips a test for an affected crate or its reverse-dep closure.
- **Tests** (`scripts/tests/`): `EnforceRolloutTests` (affected⇒RUNS,
  not-affected⇒SKIPS via the exact shard-guard membership rule, ci-full⇒full,
  nightly⇒full, fail-safe trigger⇒full, selector-error⇒full, a mutation check on
  the quoted-needle guard) + wiring inspection (enforce-default, ci-full override,
  label-toggle triggers, the nightly backstop job).

**Shipped** (sq-va7at, PR #1526): the §6.1 *selection-bug alarm* now correlates
failed nightly jobs with suspect landed PRs and auto-files a deduplicated issue.

## 7. Firm positions (decision record)

- **P1 — selector is stdlib-only Python** (`scripts/ci_select.py`), not a
  Rust xtask and not a third-party action: zero build latency in the pre-job,
  JSON-native, unit-testable via `unittest`, covered by the `scripts/**`
  full-run trigger for self-changes. *Rejected:* `dorny/paths-filter`-style
  path lists (no dependency-closure semantics — unsound for a workspace);
  guppy/`determinator` (right semantics, prior art worth mirroring in the
  golden tests, but drags a Rust build into the pre-job and hides the rule
  set we need to be explicit and auditable).
- **P2 — package-level dependency edges, all kinds, features ignored**:
  superset of every feature-resolved graph ⇒ conservative (§3.3).
- **P3 — ownership covers all in-repo path packages**, longest-prefix match.
- **P4 — whole-crate granularity**; no sub-crate path refinement. Marginal
  extra savings, real soundness risk.
- **P5 — static matrix + job-level `if:` guards**, not dynamic matrix
  generation: skipped jobs cost no runner slot, required-check names survive
  as `skipped` (= satisfied), and the workflow diff is a guard per job rather
  than a rewrite. Dynamic matrices leave required checks "expected"/missing.
- **P6 — fail-closed at every layer** (§4.3): selector error ⇒ full; empty
  outputs ⇒ run; select hard-failure ⇒ red aggregator.
- **P7 — phase 1 scopes only the wide lanes**; singleton lanes (coverage,
  CodeQL, fuzz, wasm, perf-gate) stay always-run until phase 2 scopes them
  deliberately (bead 6). Minimizes the initial soundness surface where the
  throughput win is smallest.
- **P8 — full combined-head validation on `merge_group`** [GPT-6-ASTRA]:
  #6048 supersedes queue selection and its ALLGREEN prefix-induction premise.
  The reusable caller forces `--full` before label/shadow handling, so every
  selection-controlled leg validates the combined head. Existing event and
  change-class guards still apply; this does not add otherwise absent legs.
  After the staged HEADGREEN flip, this establishes the group-head property,
  not greenness of every intermediate squash commit on `main`. PR-head selection
  remains enabled; see the [rollout trade-off](../docs/branch-protection.md#other-settings).
- **P9 — the SAFE list starts empty** and only audit-proven entries join it.
- **P10 — enforce only after the shadow window** (§6.4); nightly full +
  `ci-full` label remain permanent backstops. *(sq-fmx4u.5, 2026-07-03:
  ENFORCEMENT FLIPPED — enforce is now the default; `CI_SELECT_MODE=shadow` is the
  report-only rollback escape hatch. See the §6 graduation note.)*

## 8. Implementation plan — child beads (disjoint)

| # | Bead | What | Tier |
|---|---|---|---|
| 1 | selector core | `scripts/ci_select.py` + unit tests: diff → ownership → reverse closure → `{mode, reason, affected}`; error ⇒ full | sonnet |
| 2 | fail-safe rules + audit | §4.1 trigger set + `ci/path-ownership.toml` + the repo-wide out-of-crate-input audit + per-rule tests | sonnet |
| 3 | matrix wiring | `select` job + `if:` guards / nextest filtersets across ci.yml, feature-matrix.yml, bench workflows + `ci-summary` semantics | sonnet |
| 4 | protection reconciliation | verify skipped-required-check semantics vs real branch protection + merge queue; shim only if needed | sonnet |
| 5 | backstops + rollout | nightly full run, `ci-full` label, `workflow_dispatch`, shadow mode + enforcement flip | sonnet |
| 6 | phase-2 scoping | fuzz/wasm/perf-gate closures; `affected = ∅` coverage skip | sonnet |

Dependency order: 1 → 2 → 3 → {4, 5} → 6. Beads carry their own acceptance
tests; see the bead records under `sq-fmx4u`.

## 9. Graduation

Once enforced, fold the operative rules (§2, §4.1, the scoped-lane table)
into the AGENTS.md gate documentation, keep the ownership map + selector as
the living source of truth, and rewrite this record's "will" into "does" — or
delete it in favor of the CI docs, per the research-record graduation rule.
