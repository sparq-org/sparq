# Feature-matrix pyramid — collapse the per-feature build+test legs into a tiered set (sq-6vshe.2)

**Status:** GRADUATED (2026-07-29, `sq-p1ccp`) — child beads 1–3 of §8 shipped the
detector, the tier-aware assembler + `check-tier` job, and the (empty) flip; bead 4 is
this record's own graduation, which moves the operative tier rules into the
`.github/workflows/feature-matrix.yml` GUARD header + the AGENTS.md gate-table row for
sq-vya1. **Those two surfaces are authoritative; this record is retained as the design
rationale + the dated evidence (§1, §9.1), not as the operative rule.** Sections written
in the anticipatory "would" describe the design as proposed on 2026-07-04; where they
differ from the living docs, the living docs win.
Authored under the proceed-and-document rule.
**Author:** Claude Fable 5 (SPARQ architect tier), 2026-07-04. [FABLE-5]
**Parent:** `research/ci-structural-speedup.md` §4 (the class analysis A–D and the
pyramid position live there; this record turns them into an implementable, evidenced
plan). **Composes with:** `research/change-based-test-selection.md` (sq-fmx4u) —
selection prunes by *crate*, this record prunes by *feature-combination*; the two
filters AND together in the same assembler.
**Implementation:** decomposed into sequenced child beads under `sq-6vshe.2` (§8).
No implementation in this PR.

---

## 1. Measured baseline (dated observation, 2026-07-03 — evidence, not a gate)

From PR #1465 (a representative `sparq-engine`-touching PR: `exec.rs`/`lib.rs`/
`reduce.rs`; head `589260b9`), via `gh api …/check-runs`:

- **72 opt-in legs defined** in `.github/feature-matrix.d/` (26 crate fragments) —
  the epic said "~50"; the real count has grown to 72.
- **58 legs ran** on this PR (change-based selection filtered 14 legs from
  unaffected crates — core/reason/reason-el/reason-ql/fedplan/substrate). This is
  the fmx4u residual the parent record predicted: an engine PR's reverse closure is
  ~the workspace, so selection cannot prune the engine-downstream legs.
- **Feature-matrix legs cost 7,778 s ≈ 130 runner-minutes — 53% of the PR's total
  244.6 runner-minutes** across all 137 checks. This is the single largest
  runner-minute block in per-PR CI.
- Per-leg wall-clock: min 28 s, median 62 s, max 349 s. The 16 `sparq-engine` legs
  alone are 248–349 s *each* (≈ 79 runner-minutes — 61% of the leg bill), because
  every leg recompiles engine + runs its test suite in that feature state.
- **Wall-clock honesty:** the PR's merge-unblocking critical path was the `gate`
  waiter at ~749 s, floored by the coverage shard (627 s) — *not* by a feature leg
  (349 s max). The pyramid therefore buys **runner-pool minutes and
  congestion-collapse headroom** (the documented failure mode where ~90-check width
  × queue depth saturates the pool and false-fails gates), not single-quiet-PR
  latency. Under pool saturation, runner-minutes *are* queue latency, so the win is
  real but indirect. No wall-clock improvement claim is made for a lone PR.
- Each leg today runs: `cargo fetch` → `cargo build -p C --features F` →
  `cargo test` (if `test: true`; all but 1 sparq-py leg are `test: true`) →
  `cargo clippy --all-targets -D warnings`.

**Premise corrections vs the epic text** (verified against the actual tree):

1. There is **no workspace `--all-features` test leg to "keep"** — the nextest
   archive deliberately avoids `--all-features` (cross-crate conflicts +
   heavy/native/network deps; see the sq-vya1 GUARD in `ci.yml`). The maximal-
   direction unification coverage that actually exists per-PR is the workspace
   `cargo clippy --all-targets --all-features -D warnings` pass (compile-level) —
   that is what T3 "keeps", plus the default-features bulk test lanes and the two
   existing feature-OFF braces (`sparq-server --no-default-features`,
   `vectorized-feature-off.yml`).
2. The **full-matrix backstop for feature legs is the push-to-main run**, not the
   nightly: `feature-matrix.yml` has no `schedule:` trigger; on `push` the select
   pre-job has no diff ⇒ `mode=full` ⇒ all 72 legs build+test after every merge.
   The ci.yml 03:17 UTC cron does not run `feature-matrix.yml` at all. The pyramid
   keeps the push-to-main run FULL — that gives a per-merge (minutes-latency)
   backstop, strictly better than the "nightly backstop" the epic asked for, at
   zero new machinery.

## 2. What each leg actually guarantees — and the evidence per leg

The parent record's class analysis (ci-structural-speedup §4.1), applied to the
real 72 legs:

| Class | Failure mode | Caught by | Needs a per-PR build+test leg? |
|---|---|---|---|
| A | incomplete feature gating — compiles today only via unification | compile/lint of the crate in that exact feature state (**check tier**) | no — check/clippy suffices |
| B | feature-*sensitive* runtime behavior — tests gated on `cfg(feature)` / behavior asserted under `cfg(not(feature))` | a **test run in that exact feature state** | **yes, for exactly the features that have such tests** |
| C | non-additivity / unification breakage (maximal direction) | workspace clippy `--all-features` + the default bulk lanes + the two feature-OFF braces | no per-feature leg |
| D | link/codegen/runtime failures in a feature state with **no** feature-gated tests (e.g. a feature that reroutes an eval path exercised by generic tests) | a **full build+test in that state** — but it need not gate every PR | push-to-main full run (per-merge backstop) |

Class D is the honest limit of the `cfg`-grep detector: a feature can change
behavior under generic tests without a single `cfg` in test code. That class is
handled by (i) the human-reviewed keep-annotation escape (§4, a leg can stay T2
with a stated reason even with zero detector hits), and (ii) the per-merge full
backstop + demotion alarm (§6).

### 2.1 Per-leg sensitivity evidence (measured 2026-07-04, repo @ `origin/main`)

**Headline finding — the epic's payoff premise is FALSIFIED by the evidence.**
The parent record expected the class-B population to be a "minority of the ~50";
measured reality: **62 of the 71 `test: true` legs have *confirmed* feature-gated
tests** (50 with explicitly `cfg(feature)`-gated `tests/` files, 7 with gated test
fns in `src/`, 5 whose feature gates a `mod` containing `#[test]` fns), 9 more are
borderline (feature code and ungated `#[test]` fns co-located in one file), and
**no leg is currently a clear check-tier candidate**. The sparq-py `arrow` leg is
`test: false`, but criterion (c) correctly retains it because
`cfg_attr(not(feature = "arrow"), forbid(unsafe_code))` protects the feature-off
build. The matrix is *not* over-specified in the class-B sense — the sq-vya1
GUARD discipline means nearly every leg exists precisely because it is the only
thing that runs a feature-gated suite (those suites compile EMPTY in the workspace
archive). The pyramid machinery below is still worth building, but for corrected
reasons: it **mechanizes the guard** (today comment-discipline only), **ratchets
against future rot-protection-only legs**, and demotes the small audited residue —
it does not collapse "~50 legs to a dozen". The big feature-matrix runner-minute
bill is mostly *irreducible at the leg-set level*; reducing it falls to the
per-leg cost levers (cache topology `sq-6vshe.5`, constant-tax consolidation
`sq-6vshe.7`, the engine split `sq-6vshe.3/.4`).

Classification key: **a** = `tests/`/`benches/` file or fn explicitly gated on
the feature; **b-direct** = gated `#[test]` fn in `src/`; **b-module** = feature
gates a `mod` that contains `#[test]` fns; **b-coloc** = feature code and
ungated `#[test]` fns co-located (audit-pending); **c-only** = `cfg(feature)`
only in non-test production code. `cfg(not)` column: where
`cfg(not(feature = …))` appears (tests / src / bin / examples).

| Leg | Features | Gated tests | cfg(not)? |
|---|---|---|---|
| sparq-arrow (arrow) | arrow | a | no |
| sparq-cli (serialize-rdf) | serialize-rdf | a | no |
| sparq-cli (terse) | terse | a | no |
| sparq-core (compact-index) | compact-index | b-coloc | yes — nested `not(any(wasm32, feature))` in `store.rs` |
| sparq-core (mmap,dict-spill) | mmap,dict-spill | a | yes-src (mmap) |
| sparq-core (jsonld) | jsonld | b-direct | no |
| sparq-core (block-bloom) | block-bloom | b-direct | yes-src |
| sparq-engine (serialize-rdf) | serialize-rdf | b-coloc | no |
| sparq-engine (cs-planner) | cs-planner | b-direct (`cfg(all(test, feature))` mod) | yes-src ×8 (`exec.rs` fallback planner) |
| sparq-engine (zk) | zk | a | no |
| sparq-engine (service) | service | a | no |
| sparq-engine (window-functions) | window-functions | a | no |
| sparq-engine (txn) | txn | a | no |
| sparq-engine (vectorized) | vectorized | a | no (+ the dedicated feature-OFF workflow) |
| sparq-engine (result-cache) | result-cache | a | no |
| sparq-engine (query-solution) | query-solution | b-coloc | no |
| sparq-engine (params) | params | b-coloc | no |
| sparq-engine (explain-json) | explain-json | b-coloc | no |
| sparq-engine (semijoin-bitmap) | semijoin-bitmap | b-coloc | no |
| sparq-engine (yannakakis) | yannakakis | b-direct | yes-src (`exec.rs`) |
| sparq-engine (streaming-serialization) | streaming-serialization | b-coloc | no |
| sparq-engine (dp-planner) | dp-planner | a | no |
| sparq-fedclient (fedclient) | fedclient | a | no |
| sparq-fedplan (adaptive-replan) | adaptive-replan | b-coloc | no |
| sparq-geo (reproject) | reproject | a | no |
| sparq-geo (geosparql_rewrite) | geosparql_rewrite | a | **yes-tests** |
| sparq-kb (validate) | validate | a | no |
| sparq-kb (query) | query | a | no |
| sparq-kb (close) | close | a | yes-bin |
| sparq-kb (literature,validate) | literature,validate | a | no |
| sparq-mcp (stdio) | stdio | a | no |
| sparq-mcp (nlq) | nlq | a | no |
| sparq-mpc (insecure-test-rng) | insecure-test-rng | a | yes-examples |
| sparq-nlq (citations) | citations | a | no |
| sparq-nlq (nlq-endpoint) | nlq-endpoint | a | no |
| sparq-policy (count-enforcement) | count-enforcement | a | no |
| sparq-policy (secprop-leftoperands) | secprop-leftoperands | a | no |
| sparq-py (arrow) | arrow (`test: false`) | **c-only** | no |
| sparq-reason-el (rbox) | rbox | a | yes-src ×3 |
| sparq-reason-el (hasse) | hasse | a | no |
| sparq-reason-el (cdomain) | cdomain | a | yes-src |
| sparq-reason-ql (experimental) | experimental | a | no |
| sparq-reason (explain) | explain | a | **yes-tests** |
| sparq-reason (rif-xml) | rif-xml | b-coloc | no |
| sparq-reason (substrate-join) | substrate-join | b-direct | yes-src ×2 |
| sparq-reason (substrate-compare) | substrate-compare | b-module | no |
| sparq-serve (change-stream) | change-stream | b-module | no |
| sparq-server (fed-descs,service,time-travel,geo,test-seams) | fed-descriptors,service,… | a | **yes-tests** (service) |
| sparq-server (audit-log,access-audit,…) | audit-log,access-audit,… | a | **yes-tests** + yes-src |
| sparq-server (tpf) | tpf | a | no |
| sparq-server (brtpf) | brtpf | a | yes-src ×5 (`http.rs` — the brTPF/TPF brace) |
| sparq-server (terse) | terse | a | no |
| sparq-server (change-stream) | change-stream | a | no |
| sparq-shacl (shacl-af) | shacl-af | a | **yes-tests** |
| sparq-solid (odrl-bridge) | odrl-bridge | a | yes-src ×2 |
| sparq-solid (count-enforcement) | count-enforcement | a | yes-src ×2 |
| sparq-substrate (rows,numeric,join,compare) | rows,numeric,join,compare | b-module | no |
| sparq-terse (vectors) | vectors | a | no |
| sparq-trust (store) | store | a | no |
| sparq-trust (delegation-prov) | delegation-prov | a | no |
| sparq-trust (secprop-vocab) | secprop-vocab | a | no |
| sparq-trust (secprop-admissibility) | secprop-admissibility | a | no |
| sparq-trust (status-list-flate2) | status-list-flate2 | b-direct | no |
| sparq-trust (status-list-flate2,did) | status-list-flate2,did | a | no |
| sparq-trust (secprop-precheck) | secprop-precheck | a | no |
| sparq-vc (did-web) | did-web | b-direct | no |
| sparq-vectors (vec-predicate,provider) | vec-predicate,provider | a | no |
| sparq-vectors (vec-predicate,filtered-ann) | vec-predicate,filtered-ann | a | yes-src |
| sparq-vectors (kge) | kge | a | yes-examples |
| sparq-vectors (compose) | compose | b-module | no |
| sparq-vectors (neuro-symbolic) | neuro-symbolic | b-module | no |
| sparq-zk (secprop-annotations) | secprop-annotations | a | no |

Five legs carry `cfg(not(feature))` **inside `tests/` files** — the feature-OFF
state has explicit assertions, so these are unconditionally T2: geo
`geosparql_rewrite`, reason `explain`, both `service`-carrying sparq-server combo
legs, shacl `shacl-af`. (`cargo hack` appears nowhere in the repo today; the
existing feature-OFF braces are the custom `vectorized-feature-off.yml` gate and
the `sparq-server --no-default-features` job.)

**Demotion candidates (the honest total):**

- **Clear (0):** none. sparq-py (arrow) is `test: false`, but it is c-only:
  `cfg_attr(not(feature = "arrow"), forbid(unsafe_code))` makes compiling the
  feature-off safety brace part of the ratchet, so it remains tier `test`.
- **Audit-pending (9, b-coloc):** sparq-core `compact-index`, sparq-engine
  `serialize-rdf` / `query-solution` / `params` / `explain-json` /
  `semijoin-bitmap` / `streaming-serialization`, sparq-fedplan `adaptive-replan`,
  sparq-reason `rif-xml`. Their co-located ungated tests also run in the default
  workspace lane; the leg's marginal signal is "the same tests, feature ON". Each
  needs a per-test-body audit (does any test exercise the gated path or change
  expectation under the feature?) before demotion — six of the nine are expensive
  `sparq-engine` legs, so this audit carries most of the achievable saving.
  Detector caveat: `compact-index`'s gate hides inside
  `cfg(not(any(wasm32, feature)))` — the detector must parse nested
  `any(...)`/`all(...)` combinators and fail-closed (classify sensitive) on any
  cfg expression it cannot parse.
- **Everything else (62): stays T2.** No demotion, no re-litigating.

## 3. The pyramid (target shape)

- **T1 — per-PR check tier (one sharded job):** every leg **demoted** out of T2
  runs `cargo clippy -p <crate> --features <F> -- -D warnings` (check-profile: no
  codegen, no link, no test execution) inside ONE job with a **shared target
  directory**, so the dependency stack under each crate compiles once, not once
  per leg. Driven by the SAME assembler (`--tier check` emission) so the leg set
  has one source of truth. Sharded 2 ways initially (`sparq-engine` legs vs rest —
  engine's frontend recompile per feature-state dominates), rebalanced from
  measured wall-clock. Covers class A for every demoted leg; keeps `-D warnings`
  lint parity with today's legs (delta: no `--all-targets` — a demoted feature has
  no feature-gated test code *by definition of the demotion criterion*, so
  test-target lints in that feature state add nothing feature-specific).
- **T2 — per-PR curated full legs:** legs whose features have feature-sensitive
  tests (class B) keep today's full build+test+clippy leg, *unchanged*, including
  every `cfg(not(feature))`-bracing leg (brTPF vs TPF, `vectorized`, …) — the
  compiled-out-assert class that already bit under `--workspace` unification stays
  braced. Membership is **evidence-ratcheted** (§4), not vibes.
- **T3 — per-PR existing lanes, untouched:** workspace default bulk test lanes,
  workspace clippy `--all-features`, `sparq-server --no-default-features`,
  `vectorized-feature-off.yml`, coverage/conformance ratchets. **This record
  changes none of them** — constraint (5): no ratchet weakens; the coverage and
  conformance floors run exactly where they run today.
- **Backstop — per-merge full matrix:** `push` to main (and `workflow_dispatch`,
  and any future `schedule` on the workflow) assembles ALL legs as full
  build+test+clippy legs — byte-identical to today's push behavior. A demoted
  leg's class-D residual surfaces within minutes of the merge that broke it, and
  trips the alarm (§6).

Selection composition (constraint: compose, don't fight): the assembler applies
**two independent AND-ed filters** — fmx4u selection (drop crates outside the
affected closure) and the tier filter (demote non-sensitive legs to T1). An
engine PR keeps its engine T2 legs but sheds engine's class-A-only legs to the
check tier; a leaf-crate PR sheds both dimensions. Both filters fail-closed to
"more legs" independently.

## 4. The detector + ratchet (evidence, not vibes — and the sq-vya1 guard, mechanized)

A small checked-in script, `scripts/feature-matrix-tiers.py` (stdlib-only Python,
same doctrine as `ci_select.py` — P1 in the fmx4u record):

- **Classifies** each fragment leg: for every feature F in the leg's set, grep the
  crate for `cfg(feature = "F")` / `cfg(not(feature = "F"))` / `cfg_attr` forms
  and attribute hits to (a) `tests/`+`benches/` files, (b) `#[cfg(test)]`/`#[test]`
  code under `src/`, (c) non-test `src/`. Hits in (a) or (b) — or **any**
  `cfg(not(feature = "F"))` anywhere in the crate — mark the leg **sensitive**
  (tier `test`). `cfg(not(…))` is deliberately over-inclusive: a distinct
  feature-OFF path means both states carry behavior worth executing.
- **Fail-closed:** any parse/IO/grep error classifies the leg **sensitive**. A
  detector bug degrades to "keep the full leg", never to a silent demotion.
- **Enforces** (run as a step of the gating `assemble feature matrix` job):
  1. a leg may carry `tier: check` only if the detector agrees it is
     non-sensitive **or** the fragment carries an explicit reviewed
     `tier-reason:` override — and a `tier: check` + sensitive-detector verdict
     is a HARD failure (the ratchet: a feature re-enters T2 the moment someone
     adds a feature-gated test, because the next PR's enforcement step reds until
     the fragment is re-promoted);
  2. **every feature with sensitive tests appears in some `test: true` leg** —
     this mechanizes the sq-vya1 GUARD that today exists only as a comment
     (feature-gated suites compile EMPTY in the workspace archive; a leg here is
     the only thing that runs them). The pyramid work *hardens* this invariant
     rather than relying on review discipline;
  3. reports (advisory) declared opt-in features that have **no** leg at all,
     against the documented SCOPE exclusion allowlist (zlib-ng, hdt/write, live,
     embeddings, mimalloc) — the completeness gap cargo-hack's `--each-feature`
     would cover; see §7 Rejected for why we don't adopt cargo-hack semantics now.
- **Demotion is explicit:** fragments gain an optional `tier:` field;
  **missing/unknown tier ⇒ `test`** (full leg). Nothing is demoted by default,
  by omission, or by a detector run — only by a reviewed fragment edit that the
  enforcement step then holds to the evidence.

## 5. Required-check reconciliation (constraint 2 — reuse fmx4u.4, invent nothing)

Verified live by sq-fmx4u.4 (ruleset 17688455): `required_status_checks` contains
exactly one entry — `ci-summary / gate`. No `opt-in <name>` leg is individually
required. Legs filtered **at assembly time spawn no check-run at all**, and the
polling aggregator (`ci_summary_gate.py`) discovers siblings by name, so an
unassembled leg is simply never waited on; the merge queue (ALLGREEN, 60-min
timeout) blocks only on `gate`. Tier demotion uses the **identical mechanism**
as selection filtering — the same assembler, the same unassembled-leg semantics,
the same `TestRequiredCheckAnchor` pins (gate job named `gate`, unguarded,
`merge_group`-triggered). The new T1 check-tier job is named without
"advisory"/"informational", so the aggregator auto-discovers it as REQUIRED — a
demoted leg's compile break still blocks the merge, via T1 instead of its old leg.
No ruleset change, no shim, no second mechanism.

## 6. Fail-safe argument (constraint 1) + the alarm

A leg leaves the per-PR set only when **all** of:

1. T1 checks the *same crate × feature set* with `-D warnings` on every PR the
   leg would have run on (class A retained, per-PR);
2. the detector proves no feature-gated test code and no `cfg(not(feature = …))`
   dependence — or a human keep-reason pins it in T2 anyway (class B retained,
   per-PR, ratcheted);
3. the push-to-main run still executes the full build+test leg per merge
   (class D residual bounded to a minutes-wide post-merge window);
4. the enforcement step reds any future drift (a new feature-gated test in a
   demoted feature blocks at the next PR).

**Alarm (sq-va7at pattern):** a push-to-main full-run failure in a leg that was
check-only on the PRs that landed since the last green main run is *prima facie*
a false demotion → auto-file a P1 bead naming the leg + suspect PRs. This is the
same correlation machinery sq-va7at builds for selection skips; extend that bead
to cover tier demotions (one alarm, two skip-sources) rather than building a
second alarm. A note has been added to sq-va7at.

**Intermediate-state safety (bead sequencing, §8):** the assembler/wiring bead
lands with ZERO fragments annotated — behavior byte-identical to today, check
tier green-empty (proven by the name-set preservation test). The demotion flip is
a separate, fragments-only PR, so at no point does a leg leave the per-PR set
before the check tier that replaces it is live and gating.

## 7. Rejected alternatives

- **`cargo hack check --each-feature` as T1** (the parent record's sketch, refined
  here): cargo-hack's `--each-feature` toggles each feature against
  `--no-default-features`, a *different* feature state from today's legs
  (`default + F`). Adopting it means triaging every crate whose features assume
  default-on siblings — real signal, wrong bead. v1 drives T1 from the SAME
  fragment leg set with byte-identical `--features` semantics (zero new feature
  states, zero triage risk, reuses the assembler's unit-tested filtering). The
  features-without-any-leg gap that `--each-feature` would close is surfaced by
  the detector's completeness report instead (§4.3); adopting cargo-hack for that
  residual is a possible follow-up once the report shows what it would add.
- **Pairwise/powerset combination legs:** rejected in the parent record (§4.2) —
  combinatorial cost for interactions Cargo doctrine already requires to be
  additive; T3 + the braces bound the residual.
- **Dropping per-feature verification to nightly entirely:** loses class A's
  per-PR locality — the matrix's real daily catch (a missing cfg-gate is by far
  the most common feature break; every demoted leg keeps per-PR compile+lint).
- **A nightly cron as the backstop** (the epic's ask): strictly worse than the
  push-to-main full run that already exists (hours vs minutes of exposure, plus
  new machinery). Kept as stated: push stays full.
- **Job-level `if:` guards per leg for tiering:** already found non-implementable
  by sq-fmx4u.3 (`matrix` context unavailable in `jobs.<id>.if`); assembly-time
  filtering is the shipped, unit-tested mechanism (see sq-fmx4u.7 erratum).

## 8. Implementation plan — child beads (sequenced, disjoint files)

| # | Bead | Files (disjoint) | Tier | Depends on |
|---|---|---|---|---|
| 1 | `sq-6g9kr` (P1) — detector + ratchet script (mechanizes the sq-vya1 guard; must parse nested `any`/`all` cfg combinators, fail-closed) | `scripts/feature-matrix-tiers.py`, `scripts/tests/test_feature_matrix_tiers.py` | sonnet | — |
| 2 | `sq-ldg8c` (P2) — assembler tier/event awareness + check-tier job + enforcement wiring (zero annotations — behavior-preserving) | `scripts/assemble-feature-matrix.py`, `scripts/tests/test_feature_matrix_assemble.py`, `.github/workflows/feature-matrix.yml` | sonnet | 1 |
| 3 | `sq-s5dvo` (P2) — THE FLIP: per-test-body audit of the 9 b-coloc legs (§2.1), then annotate only audit-cleared legs `tier: check`; mutation demo + before/after counts | `.github/feature-matrix.d/*.yml` (26 fragments) | sonnet (+ escalated review of the demotion list) | 2 |
| 4 | `sq-p1ccp` (P2) — docs graduation: GUARD header + AGENTS.md gate docs describe the tier system as shipped | `AGENTS.md`, `.github/workflows/feature-matrix.yml` (comments) | haiku | 3 |

The chain is deliberate: each intermediate state is fail-safe (§6). Bead 3 is the
only soundness-sensitive PR (it is the demotion); its review is escalated per the
arm-gate doctrine, and its audit may legitimately return "demote nothing" — that
is a valid outcome, recorded here as such.

## 9. Estimated reduction (counts, honest)

Counts, not percentages, against the §1 engine-PR baseline (58 legs ran,
≈130 runner-minutes):

- **Floor (certain):** 0 legs demoted — leg count remains 58. The durable value
  at the floor is the mechanized guard + the ratchet.
- **Ceiling (if the per-test audit clears all 9 b-coloc legs):** 9 legs demoted,
  of which 6 ran on the baseline PR (the 6 sparq-engine legs;
  core/fedplan/reason legs were already selection-filtered there) —
  roughly **28–30 of the 130 leg runner-minutes** move into a check-tier job
  costing an estimated 3–6 min (clippy-profile, shared target dir; measured at
  implementation, not promised). Leg count 58 → 52 + 1–2 check shards.
- **What this is NOT:** the epic's hoped-for "~50 legs → a dozen". That premise
  assumed most legs were compile-rot protection; the evidence (§2.1) shows 62/71
  carry real feature-gated test suites that nothing else runs. Their bill is
  addressed by the per-leg cost levers (`sq-6vshe.5/.7`, and structurally
  `sq-6vshe.3/.4`), not by leg-set surgery.
- **Forward value (unquantified, honest):** every future feature added for
  rot-protection only (no gated tests) now lands as `tier: check` — the leg set
  stops re-widening by default, and the enforcement step stops the sq-vya1
  silent-gap class (a feature-gated suite with no leg) mechanically instead of
  by review discipline.

### 9.1 Flip audit observation (2026-07-26)

No b-coloc leg was audit-cleared or demoted in this flip; the nine candidates
remain audit-pending. The proposed sparq-py `arrow` demotion was rejected by the
detector's criterion (c): its feature-off `forbid(unsafe_code)` cfg attribute is
intentionally sensitive. The measured before/after count is therefore
**0 → 0 check-tier legs**.

The detector mutation demonstration remains pinned in
`TestMutationFailOpen::test_mutation_check_divergence`: the fail-open mutant
classifies an unreadable or missing source tree non-sensitive, while the
production fail-closed path classifies the same input sensitive. The
tier-enforcement tests additionally make a sensitive `tier: check` leg fail
invariant (1), demonstrating that adding a feature-gated test under a demoted
feature turns the ratchet red.

## 10. Graduation

As the beads land: fold the tier definitions + the detector ratchet rule into the
`feature-matrix.yml` GUARD header and the AGENTS.md gate documentation (bead 4);
record the measured before/after leg counts from the flip PR as a dated
observation here; then rewrite this record's "would" into "does" or delete
sections in favor of the living docs, per the research-record graduation rule.

### 10.1 Graduation completed (2026-07-29, `sq-p1ccp`)

Done, and deliberately WITHOUT restating the rules in two places:

- **Tier definitions + the ratchet rule** now live in the `feature-matrix.yml` GUARD
  header (the `[OPUS-5] sq-p1ccp` block, immediately after the original sq-vya1 guard)
  and in the AGENTS.md gate-table row for "an opt-in cargo feature, or a test behind a
  default-OFF feature". Both name `scripts/feature-matrix-tiers.py --enforce` as the
  step in the gating `setup` job, spell out enforcement invariants (1) and (2), the
  fail-closed detector, and the `tier: test` (default) vs `tier: check`
  (PR clippy-only → full build+test on push-to-main) split.
- **§4's invariants key on the `(crate, feature)` PAIR, not the bare feature name** —
  a review finding on the graduation PR. Cargo feature names are crate-LOCAL, and 15 are
  currently shared across crates (`arrow`, `service`, `templates`, …), so the first cut's
  bare-name keying let a `test: true` leg for `F` in crate A silently satisfy a sensitive
  `F` in crate B — the exact silent gap the guard exists to catch. Pair-keying exposed one
  live instance (`sparq-py`/`arrow`, masked by `sparq-arrow`/`arrow`), so invariant (2)
  gained a narrow escape: a written `test-reason:` on the leg, for coverage that genuinely
  cannot be a `cargo test` leg. Regression cover:
  `scripts/tests/test_feature_matrix_tiers.py::TestCrossCrateFeatureNameCollision` (two
  crates, one shared feature name, only the unrelated crate `test: true`).
- **Measured before/after counts:** already recorded above as the dated §9.1 observation
  — **0 → 0 check-tier legs**. Bead 3's per-test audit cleared nothing, which §8
  anticipated as a legitimate outcome, so the leg count is unchanged and the §9 "floor"
  case is what shipped: the durable value delivered is the mechanized guard + ratchet,
  not a leg-count reduction. The §9 ceiling (9 legs demoted) remains UNREALIZED; the
  nine b-coloc candidates are still audit-pending.
- **Not rewritten to "does":** §§1–7 are kept verbatim as the dated design rationale and
  evidence. The Status header above now marks the record graduated and points at the
  living docs as authoritative, which is the cheaper half of the "rewrite or delete"
  choice and avoids a second copy of the operative rule drifting out of sync.
