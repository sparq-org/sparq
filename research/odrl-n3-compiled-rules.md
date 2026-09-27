# Design — ODRL evaluation as N3 inference rules, with build-time rule compilation

> 🤖 **SPARQ agent** — decomposition design record for maintainer issue
> [#1582](https://github.com/sparq-org/sparq/issues/1582) (epic `sq-zgbso`). [FABLE-5]
> DESIGN only: this PR changes no crate code. It verifies the issue's premise against the
> actual code, states the feasibility envelope honestly, and decomposes the work into
> measurement-gated child beads (`sq-zgbso.1`–`.6`).

**Status:** phases 1–3 LANDED; `.4`–`.6` open. **Epic:** `sq-zgbso` (issue #1582).
**Gate:** everything past the first bead was blocked on the `sq-zgbso.1` spike verdict —
that spike has since landed and `.2`/`.3` were built on it (§9).
**Read §9 first** — it reconciles the original plan below with what is actually in the
tree, and corrects three statements the plan got wrong.

**The ask, condensed (issue #1582):** (1) move ODRL evaluation out of hand-written Rust
into Notation3 inference rules, the way WAC and ACP evaluation are done; (2) HARD
constraints — no performance degradation, no build-size increase; (3) likely "compile"
the N3 at build time rather than parse at run time, with an index-layer representation
(pre-indexed URIs with attached ids) that could yield a NET improvement for WAC/ACP
evaluation too; (4) if it works, follow on with RDFS and OWL-RL rules as compiled N3.

---

## 0. Premise check — the verified current state (honesty first)

The maintainer hedged: WAC/ACP were N3-rule-evaluated "when I checked last week, this may
have changed." Verified against `origin/main` (`cef945af`, 2026-07-05): **it has not
changed — WAC and ACP evaluation ARE N3 inference rules today, and ODRL is not.**

1. **WAC/ACP = N3 rules, embedded at build time, parsed + evaluated at RUN time.**
   `crates/sparq-solid/rules/{common,wac,acp-a,acp-b,acp-c}.n3` (~460 lines of rules) are
   embedded via `include_str!` (`src/materialize.rs:24-28`) and run through
   `sparq_reason::reason_n3` — WAC as one stratum, ACP as three stratified calls
   (accepts → rejections → grants, design `research/solid-access-control-design.md` §3.5).
   So the *rule text* costs build size already; what happens per `materialize_*` call is
   the interesting part (§1).

2. **ODRL = hand-written Rust.** `crates/sparq-policy` (`eval.rs` 1 854 lines +
   `compare.rs` 607 + `model.rs` 386 + `parse.rs` 576) evaluates
   Permission/Prohibition/Duty with constraint operators to a fail-closed decision, and
   `crates/sparq-solid/src/odrl_bridge.rs` (2 006 lines, behind the default-OFF
   `odrl-bridge` feature) maps decisions into the same `<urn:sparq:auth>` view the
   WAC/ACP rules write — including *conditional* grants (live-clock `TimeWindow`s from
   `odrl:dateTime`, `auth:exceptMatcher` noneOf carve-outs from `recipient neq`).

3. **The original design actually proposed N3-based ODRL.**
   `research/feature-research-odrl-policy.md` candidate #1 specified the ODRL evaluator
   "over RDF, on the existing N3/EYE reasoner", mirroring SolidLab's
   [ODRL-Evaluator](https://github.com/SolidLabResearch/ODRL-Evaluator) (N3 + EYE). The
   implementation went hand-written Rust instead. Issue #1582 is a return to the recorded
   design direction, now with a perf/size discipline attached — the ask is coherent with
   the estate, not a novelty.

4. **The "index layer" intuition has real substance in the code.** The N3 engine
   (`crates/sparq-reason/src/n3/`, ~5 400 lines) runs its semi-naive fixpoint at the
   **string term level** (`Term::Iri(String)`, `n3/model.rs`) and interns to dictionary
   ids only at the end. Worse, the materialize pipeline round-trips through *text* in
   both directions: `assemble_input` serializes the access-control graphs **to N3 text**,
   `reason_n3` re-parses that text, and ACP re-serializes each stratum's closure back to
   text for the next stratum (`closure_to_n3`, `materialize.rs:226`). Facts already live
   as `[Id; 3]` in a `Dict` inside `Graph` — the round trip is pure overhead, and rules
   with pre-interned URI ids evaluated directly over `[Id; 3]` facts is exactly the
   maintainer's "URIs pre-indexed and ids attached" representation.

5. **An id-level rule substrate already exists and must be reused, not duplicated.**
   `crates/sparq-substrate` (epic `sq-qonbz` under `sq-6tykl`,
   `research/shared-eval-substrate.md`) ships monomorphic id-level hash-join kernels
   (`build_table`/`probe_emit`) that the SPARQL engine AND the RDFS materialiser already
   share (`sparq-reason/src/substrate_join.rs`, opt-in `substrate-join` feature). A
   compiled-rule evaluator is a semi-naive join loop over `[Id; 3]` — it must drive these
   kernels, not hand-roll a third join implementation.

6. **Known interaction:** `sq-s5tkx` — a pre-existing 8-test failure in the
   `--all-features` `odrl_bridge` suite (count-enforcement feature combination), verified
   on `origin/main` and NOT caught by CI (no `--all-features` lane for sparq-solid). The
   fix is a separate bead; every differential in this plan therefore pins
   `--features odrl-bridge` (not `--all-features`) as its baseline.

**Net:** the issue's premise is correct on all counts. The epic is sound; the open
questions are quantitative, so the plan below is measurement-gated.

---

## 1. Where the headroom actually is — anatomy of a `materialize_*` call

Per WAC materialize call today (`materialize.rs:103`):

```text
graphs ──assemble_input──▶ N3 TEXT ──parse──▶ Term(String) facts ─┐
rules/*.n3 (include_str!) ──────────parse──▶ Term(String) rules ──┤
                                                                  ▼
                              string-level semi-naive fixpoint (run_closure)
                                                                  ▼
                              intern_closure ──▶ [Id;3] ──filter──▶ <urn:sparq:auth>
```

ACP does this three times, re-serializing each closure to text in between. Costs that a
compiled path can remove, in decreasing expected size (to be MEASURED by the spike, not
assumed):

| # | Cost today | Removed by |
|---|------------|-----------|
| 1 | Facts: graph → N3 text → re-parse → `String` terms, per call (and 3× for ACP incl. inter-stratum re-serialization) | id-level fact entry (facts stay `[Id; 3]` from the source `Graph`/`Dict`) |
| 2 | Fixpoint joins hash on `String` terms | id-level fixpoint over `u32` ids on `sparq-substrate` join kernels |
| 3 | Rule text: parsed on every call | build-time compilation to an IR (parse count per process: 0) |
| 4 | N3 parser linked into the runtime path | build-time compilation (parser becomes a build-dependency) |

Item 3 alone (the literal "parse the rules once") is small — the rule text is a few KB
against fixture inputs of ~10³ graphs. **The real candidate win is items 1–2**, which is
precisely the index-layer representation the issue proposes. This is why the spike
(§6) measures the *parse fraction* of the existing pipeline before anything is built: if
items 1–3 are a small share of materialize wall-clock, the compiled path has no perf case
and only the build-size/maintainability arguments remain — the verdict must say so.

Build-size ledger (the second HARD constraint), what moves in each direction:

- **Removed** from the runtime artifact: the N3 text parser's reachable code (if the
  compiled path makes it a build-dependency only) + the rule text bytes.
- **Added:** the compiled IR (const tables or an `include_bytes!` blob) + the id-level
  evaluator code + whatever `build.rs` machinery forces onto compile time (build-deps do
  NOT enter the runtime artifact).
- Verdict rule (bead `sq-zgbso.4`): if the adds exceed the removes on measured release
  artifacts, STOP and report — do not land.

`sparq-solid` and `sparq-reason` are outside the wasm bundle and outside `sparq-core`/
`sparq-engine`, so the `wasm_bundle_bytes` ratchet is not at risk; the size measurement
targets the sparq-solid/sparq-reason release artifacts and a representative downstream
binary.

---

## 2. Constraints (from the issue + the house architecture)

1. **No performance degradation** — enforced per-bead by result-equivalence + a measured
   before/after ratio on `examples/bench.rs`-class harnesses (numbers reported in PR
   bodies as non-canonical work-box measurements, never baked into docs/tests).
2. **No build-size increase** — measured release-artifact deltas, reported per PR; the
   compiled-path bead fails closed on this gate (§1 ledger).
3. **Opt-in architecture** — `sparq-core`/`sparq-engine` untouched; the compiled-rule
   evaluator is a default-OFF `compiled-rules` feature in the already-opt-in
   `sparq-reason` (the `substrate-join` pattern); `odrl-bridge` stays default-OFF.
4. **Fail-closed access control** — a missing/unmappable rule construct must yield NO
   grant; every migration bead's acceptance test is a Rust-vs-N3 decision differential,
   and over-granting is the failure mode the differentials are designed around.
5. **Answer-safety of the existing suites** — WAC/ACP conformance + the ACP differential
   oracle + the odrl_bridge corpus are the equivalence oracles; they pass unchanged at
   every step.

---

## 3. What can be N3 and what cannot (honest scoping)

**Can (stateless core):** Permission/Prohibition matching (target/assignee/action →
`action_to_mode`), logical constraint combinators (`and`/`or`/`xone` — expressible with
the same stratification discipline ACP already uses), and the stateless constraints the
bridge *faithfully maps* today: `odrl:dateTime` windows (the rules emit the same
conditional-grant `TimeWindow` facts the bridge writes; the session layer keeps doing the
live-clock re-check) and `recipient`/`assignee neq` carve-outs (same `auth:exceptMatcher`
noneOf facts). The N3 builtin needs are exactly what `rules/wac.n3`/`acp-*.n3` already
use: scoped `log:notIncludes`, `string:encodeForUri`, `string:concatenation`, `log:uri`.

**Cannot (stays Rust):**

- **Stateful `odrl:count` enforcement** (`count-enforcement` feature): a usage-counter
  store with atomic exercise-and-increment is *mutation*, not inference. A pure rule
  cannot decrement a budget. The counter stores (`count.rs`/`count_file.rs`/
  `count_backend.rs`) and the exercise path remain Rust regardless of the spike verdict.
- **The secprop surface** (`sparq-policy/src/secprop.rs`) — out of scope for this epic.
- **Constraint operators without a faithful stateless mapping** — the bridge already
  classifies faithful vs unmappable; unmappable stays deny (fail-closed), same as today.

**Consequence:** "move ODRL evaluation into N3" honestly means *the stateless decision
core*, with the bridge's stateful/window/exception plumbing intact. That is still the
bulk of `eval.rs`/`compare.rs` semantics, and it is the same split SolidLab's
N3-based evaluator makes.

---

## 4. Options considered

| Option | Perf | Build size | Verdict |
|--------|------|-----------|---------|
| **(a) Status quo** — ODRL stays Rust | baseline | baseline | Rejected as the end state (issue #1582 asks for the migration; two evaluation stacks for one auth view is the real maintenance cost) — but it IS the fallback if the spike returns NO-GO on both axes. |
| **(b) Parse-once (`OnceLock`) rule caching, text facts** | removes item 3 only (§1) — facts still round-trip through text per call | unchanged | Rejected as the target: does not touch the dominant costs (items 1–2) and delivers none of the index-layer upside. Kept in the spike as a cheap comparison point. |
| **(c) Build-time compile → id-level IR + substrate-join evaluator** (chosen) | removes items 1–4; joins go id-level on existing kernels | parser leaves the runtime path; IR replaces rule text — measured ledger, fail-closed gate | **Chosen, gated on the spike.** Matches the issue's index-layer proposal; reuses `sparq-substrate` instead of adding a third join; follows the `substrate-join` opt-in pattern. |
| **(d) Proc-macro N3 compilation** | same as (c) | same as (c) plus a proc-macro crate in every downstream build graph | Rejected: heavier build infra, worse incremental-build behavior than a `build.rs` with the parser as a plain build-dependency, no capability delta. |
| **(e) External EYE/eye-js at runtime** | unknown | large new dependency | Rejected: contradicts the native-estate direction; sparq already ships its own N3 engine with W3C conformance coverage. |

Within (c), the evaluator lives in **`sparq-reason`** (feature `compiled-rules`,
default-OFF) rather than a new crate: it shares the N3 model/builtins and the
`substrate-join` wiring that are already there, and `sparq-reason` is itself opt-in
(a dependency of nothing in the core). The compile *front-end* (N3 text → IR) also lives
there, so `sparq-solid`'s `build.rs` consumes it as a build-dependency.

---

## 5. Decomposition — six disjoint, spike-gated child beads

Phased plan; every bead carries `crate` / `model_tier` / `invariant` / `acceptance_test`
in its body, plus an explicit file-area so disjointness is auditable. No two beads that
can run concurrently touch the same file or the same crate.

| Bead | Phase | Crate | Tier | One-line scope | File area |
|------|-------|-------|------|----------------|-----------|
| `sq-zgbso.1` | 1 — spike (GATE) | sparq-solid | opus | ONE ODRL case as N3: decision differential vs the Rust path + timing ratio + WAC/ACP parse-fraction profile + build-size delta; measurement-only | `rules/odrl-spike.n3`, `examples/odrl_n3_spike.rs`, `Cargo.toml` (`[[example]]`) |
| `sq-zgbso.2` | 2 | sparq-solid | opus | Stateless ODRL core as N3 strata via runtime `reason_n3` (the WAC/ACP pattern); full-corpus Rust-vs-N3 differential; count stays Rust | `rules/odrl-*.n3`, `src/odrl_bridge.rs`, `tests/odrl_n3_differential.rs` |
| `sq-zgbso.3` | 2 | sparq-reason | opus | Id-level compiled-rule evaluator behind default-OFF `compiled-rules`: pre-interned-URI rule IR + compile front-end + semi-naive fixpoint on `sparq-substrate` joins | `src/n3/compiled.rs`, `src/n3/mod.rs` (hook), `Cargo.toml`, `tests/compiled_equivalence.rs` |
| `sq-zgbso.4` | 3 | sparq-solid | opus | `build.rs` compiles `rules/*.n3` at build time; `materialize_wac/acp` flip to the compiled evaluator with id-level fact entry; perf + size HARD gates | `build.rs`, `src/materialize.rs`, `src/loader.rs`, `Cargo.toml` |
| `sq-zgbso.5` | 3 | sparq-solid | haiku | Mechanical flip of the ODRL strata to the compiled path; rerun the differential | `src/odrl_bridge.rs` |
| `sq-zgbso.6` | 4 — spike (GATE) | sparq-reason | opus | RDFS ruleset as compiled N3 vs the hand-optimized id-level materializer: equivalence + honest ratio; verdict gates OWL-RL-as-N3 | `rules/rdfs.n3`, `examples/rdfs_n3_spike.rs`, `Cargo.toml` (`[[example]]`) |

Dependency edges (only where ordering is real):

```text
sq-zgbso.1 (spike verdict)
 ├─▶ sq-zgbso.2 (ODRL-as-N3, needs the ODRL GO)
 └─▶ sq-zgbso.3 (compiled evaluator, needs the compilation GO)
        sq-zgbso.2 ─┐
        sq-zgbso.3 ─┴─▶ sq-zgbso.4 (build-time flip; .2 edge is same-crate
                         serialization — lifts if .2 closes as won't-do)
                         sq-zgbso.4 ─▶ sq-zgbso.5 (ODRL compiled flip)
                         sq-zgbso.4 ─▶ sq-zgbso.6 (RDFS follow-on spike)
```

Parallelism audit: the only concurrently-runnable sets are `{.2 (sparq-solid),
.3 (sparq-reason)}` and `{.5 (sparq-solid), .6 (sparq-reason)}` — one bead per crate,
zero shared files. `Cargo.toml`/`odrl_bridge.rs` overlaps across phases are strictly
sequenced by the dep edges. Only `sq-zgbso.1` was dispatchable when this plan was
written, by design; `.1`–`.3` have since landed and `.4` is the current frontier (§9).

**Verdict semantics of the spike (`sq-zgbso.1`):** two independent GO/NO-GO calls —
(i) *ODRL-as-N3*: correctness parity holds and the runtime ratio is acceptable given
WAC/ACP already pay the same evaluator (gates `.2`); (ii) *build-time compilation*: the
parse/serialize fraction of materialize wall-clock is large enough that compilation can
plausibly pay for itself (gates `.3` → `.4` → `.5`/`.6`). A split verdict is legitimate:
e.g. ODRL-as-N3 GO on the runtime path with compilation NO-GO leaves the estate exactly
as coherent as WAC/ACP today (option (b) as consolation is then worth a follow-up bead).

---

## 6. Measurement plan (as written in 2026-07; items 1–2 have since been executed — §9)

Stated up front so downstream `verify` is mechanical and nobody invents a number. All
wall-clock figures are best-of-N on the work box, reported in bead comments + PR bodies
as **non-canonical** ratios; nothing lands in docs, tests, or `bench/perf-baseline.json`.

1. **Spike (`sq-zgbso.1`):**
   - Decision-equivalence: N3-derived grant triples == Rust-bridge grant triples on the
     spike fixture, including a deny case (fail-closed direction exercised).
   - Timing ratio: N3 evaluation vs `sparq-policy` Rust evaluation on the same case.
   - Parse-fraction profile of the EXISTING WAC/ACP materialize (proxy: `reason_n3` on
     rules-only text vs facts+rules on `wac_fixture`/`acp_fixture`) — the compiled-path
     headroom number.
   - Build-size delta of embedding the spike rules (release artifacts, with/without).
2. **Compiled evaluator (`sq-zgbso.3`):** closure set-equality vs `reason_n3` on the
   WAC/ACP rule corpus; feature-OFF build byte-identical.
3. **Build-time flip (`sq-zgbso.4`):** auth-view triple-set identity on the full WAC/ACP
   conformance + differential-oracle suites; materialize before/after ratio on
   `examples/bench.rs`; release-artifact size ledger (§1) — fail-closed on both HARD
   gates.
4. **RDFS spike (`sq-zgbso.6`):** closure equivalence vs `materialize(Profile::Rdfs, …)`
   on the entailment-conformance fixtures, then the wall-clock ratio vs the hand-optimized
   id-level path.

---

## 7. Risks and honest notes

- **The RDFS/OWL-RL follow-on has a much higher bar than ODRL.** ODRL-as-N3 competes
  with a straightforward Rust evaluator over small policy graphs; RDFS/OWL-RL-as-N3
  competes with hand-optimized id-level materializers that already drive substrate join
  kernels (`rdfs.rs`, `owl.rs`, `substrate_join.rs`). The honest expectation is that the
  compiled-N3 path's win there, if any, is single-source-of-truth rule text and less
  bespoke Rust — not speed. `sq-zgbso.6` is scoped as a spike with a mandatory measured
  ratio for exactly this reason; OWL-RL work is created only on its GO.
- **Access-control surface ⇒ maintainer-armed.** Beads `.2`, `.4`, `.5` change how
  grants are derived. The differentials make over-granting detectable on the corpus, but
  off-corpus divergence is the residual risk — these PRs should be maintainer-reviewed,
  not fleet auto-armed. (No ZK/MPC claims are involved anywhere in this epic; the
  WAC/ACP/ODRL layer is classical access control, not a cryptographic guarantee.)
- **`sq-s5tkx`** (pre-existing 8-test `--all-features` odrl_bridge failure): all
  differentials in this plan pin `--features odrl-bridge`. The bug fix is a separate
  bead; once fixed, extending the differential to the count-enforcement combination is a
  cheap follow-up.
- **Stratification discipline is load-bearing.** The N3 engine's negation-as-failure
  never retracts, so ODRL strata must keep every negated predicate complete before its
  stratum runs (the ACP §3.5 lesson). `xone`/prohibition-precedence shapes need the same
  care; the differential corpus includes prohibition-wins cases to catch a mis-cut.
- **Substrate reconciliation, not duplication.** The compiled evaluator (`sq-zgbso.3`)
  is required to drive `sparq_substrate::join` kernels — the same seam the engine and
  the RDFS materialiser share (epic `sq-qonbz`). If the kernels' shape doesn't fit the
  rule fixpoint, the bead escalates rather than forking a third join.
- **Builtin coverage is a scoped subset.** The compiled path supports exactly the
  builtins the access-control rules use (`log:notIncludes`, `string:encodeForUri`,
  `string:concatenation`, `log:uri`) — not full N3. Full-N3 conformance stays with the
  text engine; the compiled evaluator documents this limit.

---

## 8. Pointers

- Issue: <https://github.com/sparq-org/sparq/issues/1582> · Epic bead: `sq-zgbso` · Children: `sq-zgbso.1`–`.6`
- Estate: `crates/sparq-solid/rules/*.n3`, `crates/sparq-solid/src/{materialize,loader,odrl_bridge,authindex}.rs`,
  `crates/sparq-policy/src/{eval,compare,model,parse}.rs`, `crates/sparq-reason/src/n3/`,
  `crates/sparq-substrate`, `crates/sparq-reason/src/substrate_join.rs`
- Prior records: `research/solid-access-control-design.md` (§3.5 stratification),
  `research/feature-research-odrl-policy.md` (candidate #1 — the N3-evaluator proposal),
  `research/shared-eval-substrate.md` (epic `sq-qonbz`)
- External prior art: SolidLab ODRL-Evaluator (N3 + EYE),
  <https://github.com/SolidLabResearch/ODRL-Evaluator>; ODRL evaluation semantics survey
  in `research/feature-research-odrl-policy.md` §1.2

---

## 9. Reconciliation (2026-07-27) — the landed state, and three corrections

Sections 0–8 are the ORIGINAL 2026-07-06 plan and are preserved as written. This section
reconciles them with `origin/main` at `d7539cf4`.

**Method + its limit (honesty):** everything below was verified by READING the tree —
file existence, feature wiring, CI legs, and call sites. **No measurement was re-run in
this pass**: the reconciliation checkout has no usable Rust toolchain, so nothing was
built, tested, or timed here. Consequently this section restates NO performance or
build-size number. The numbers from the executed beads live where the house rules put
them — the PR/bead bodies of #1601 and #1619, as non-canonical work-box figures — and
should be read there, not inferred from this doc.

### 9.1 What landed

| Bead | State | Verified evidence in-tree |
|------|-------|---------------------------|
| `.1` spike | **LANDED** (#1601) | `crates/sparq-solid/rules/odrl-spike.n3`, `examples/odrl_n3_spike.rs`, `tests/odrl_n3_spike.rs` |
| `.2` stateless ODRL core as N3 | **LANDED** (#3458, incl. its pre-merge fail-open fixes) | `rules/odrl-{a0,a,b,c,d}.n3`, `odrl_bridge::materialize_odrl_n3`, `tests/odrl_n3_differential.rs`, `tests/odrl_n3_failopen_regression.rs` |
| `.3` id-level compiled evaluator | **LANDED** (#1619) | `sparq-reason/src/n3/compiled.rs`, `tests/compiled_equivalence.rs`, `examples/compiled_rules_bench.rs`, feature `compiled-rules` + its `.github/feature-matrix.d/sparq-reason.yml` leg |
| `.4` build-time flip | **OPEN** | no `crates/sparq-solid/build.rs`; `src/materialize.rs` still calls `reason_n3`/`reason_n3_stratified` over assembled TEXT |
| `.5` ODRL compiled flip | **OPEN** | `materialize_odrl_n3` still runs the text engine (§9.4) |
| `.6` RDFS-as-compiled-N3 spike | **OPEN** | no RDFS N3 ruleset or spike example under `sparq-reason` |

So **the deliverable named in the issue title — "feasibility design + measured prototype" —
is done** (#1590 design, #1601 measured spike). What remains open is the *build-time
compilation* half of the epic: `.4` (the flip that would actually realise the win `.3`
built the machinery for), then `.5`/`.6`. `.3` shipped an evaluator that, as of this
commit, **nothing in `sparq-solid` calls** — the compiled path is built and
equivalence-tested, but is not yet on any production path. That gap is `.4`, and it is
the single highest-value remaining bead in the epic.

### 9.2 Correction 1 — §3's "Can" list overstates what the N3 path actually supports

§3 scoped the N3-able core as target/assignee/action matching, `and`/`or`/`xone`
combinators, `odrl:dateTime` windows, and `recipient`/`assignee neq` carve-outs. The
landed path is materially narrower. Per the `materialize_odrl_n3` rustdoc and the rule
files, the decision-EQUIVALENT scope is: `odrl:dateTime` **lteq/gteq only**, canonical
UTC lexical forms only; **`odrl:recipient`** eq/neq (not `assignee`); at most **one**
`odrl:or`/`odrl:and` per rule, with each constraint node carrying exactly **one** operand
tuple. **`odrl:xone` is not evaluated at all**: there is no `xone` satisfaction rule in
any stratum (only `or` in A and `and` in C), the stratum C/D rules explicitly negate it
(`log:notIncludes { ?r odrl:constraint ?lc . ?lc odrl:xone ?x }`) so a permission carrying
one yields no grant, a prohibition carrying one is an explicit `Err` refusal, and
`rules/odrl-a0.n3` additionally marks nodes combining `xone` with `and`/`or` as
`odrlx:Ambiguous`.

The safety framing also changed, for the better: outside that scope the landed claim is
not "equivalent" but **"never more permissive"** than the Rust path — unsupported
constructs on a permission yield no grant, and on a prohibition (where silently dropping
a deny would widen access) they refuse the whole call. That is the fail-closed direction
§2.4 asked for, stated more precisely than the plan did.

This narrowing is not a defect; it is what the fail-open review found.
`tests/odrl_n3_failopen_regression.rs` records two confirmed fail-OPEN defects that the
review of PR #3458 found by execution in the `.2` path and fixed before merge — a
multi-operand constraint node satisfied by "some operand" where "every operand" was
required, and a conflict-refusal guard that was never called. Both granted access the
Rust path denied. **The lesson for `.4`/`.5`: on this surface a differential corpus is
necessary but was not sufficient — both defects needed adversarial review to surface.**

### 9.3 Correction 2 — the N3 path depends on the Rust evaluator; it does not replace it

The epic's framing is "move ODRL evaluation out of hand-written Rust into N3". As landed,
that has not happened and cannot yet: `materialize_odrl_n3` **calls into `sparq-policy`
on every invocation** — `parse_policy_str` to re-parse the policy into the Rust model,
and `conflict_admissibility` (via `refuse_unimplementable_conflict`) to decide refusal.
A policy the Rust evaluator cannot parse is refused by the N3 path too.

This is a deliberate and sound choice — sharing the admissibility verdict is exactly what
makes the two paths refuse the same policies, and it was one of the two fail-open fixes.
But it means §4's rejection of option (a) ("two evaluation stacks for one auth view is
the real maintenance cost") describes the *current* estate: there are now two stacks, and
the N3 one sits on top of the Rust one rather than displacing it. `crates/sparq-policy`
is not on a retirement path and should not be treated as one.

### 9.4 Correction 3 — the ODRL N3 path pays the text round trip 5×, not 1× or 3×

§1's cost anatomy was written for WAC (one `reason_n3` call) and ACP (three strata).
`materialize_odrl_n3` runs **five sequential `reason_n3` calls** (strata A0→A→B→C→D),
each constructing a **fresh `Dict`** and each re-serializing the previous stratum's whole
closure back to N3 text via `closure_ids_to_n3` for the next one to re-parse.

This strengthens the §1 items-1–2 case rather than weakening it: the ODRL path is the
*most* round-trip-heavy consumer in the estate, so it — not WAC — is the workload where a
compiled id-level evaluator has the most headroom to recover. It also reorders the plan's
economics: §5 scheduled `.5` (the ODRL compiled flip) as a cheap haiku-tier mechanical
follow-on to `.4`. Five fresh-`Dict` strata make it the bead with the largest expected
win, and re-tiering it upward is worth considering when `.4` lands.

### 9.5 Open question for the maintainer — the `sparq-policy` disposition

The epic has no recorded answer to what ultimately happens to `crates/sparq-policy`'s
hand-written evaluator (`eval.rs` 1 955 + `compare.rs` 620 lines). Three coherent end states,
and the choice is a maintainer call, not an agent one:

1. **Rust stays the default, N3 stays opt-in and narrower** (the status quo). Honest and
   safe; permanently two stacks.
2. **N3 becomes the default once its scope reaches parity**, Rust demoted to the
   differential oracle. This is what the epic implies, but §9.2 shows the scope gap is
   wide (xone, all six dateTime operators, multi-operand nodes, `assignee`), and §9.3
   means the parse + conflict-admissibility core stays Rust regardless.
3. **Deliberately keep both, permanently**, with the Rust path as the reference oracle —
   arguably the right answer on an access-control surface, where a second independent
   implementation is a feature, not debt.

Nothing in the tree commits to any of these. Recording the question is the point;
answering it should precede any further scope-widening of the N3 rules.

### 9.6 Not re-verified in this pass

- `sq-s5tkx` (§0.6 / §7 — the pre-existing 8-test `--all-features` `odrl_bridge`
  failure): **not re-checked**, no toolchain. `.github/feature-matrix.d/sparq-solid.yml`
  still has no `--all-features` leg for the crate (legs are per-feature), so the
  condition the bead describes would still be invisible to CI.
- The `.1`/`.3` timing and build-size figures: not re-measured; see the PR bodies.
- Whether the current test suites are green: not run.
