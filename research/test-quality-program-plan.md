# Test-quality program plan — prioritized per-crate coverage + mutation decomposition (sq-qcnn)

> 🤖 SPARQ agent design record (architect stage, Claude Fable 5). Decomposition of epic
> **sq-qcnn** ("cargo-mutants ratchet + raise per-crate line-coverage floors to ≥90%
> core-logic") into a prioritized, **disjoint** (one crate per bead) implementation plan the
> fleet can drain in parallel. DESIGN + DECOMPOSE only — no tests are implemented here.
> Companion records: `research/test-coverage-decomposition.md` (the earlier sq-bif register),
> `research/differential-testing-value-level.md` (the sq-qcnn.2 diff-test lane — **disjoint**
> from this plan; its open children sq-qcnn.5–.10 are not duplicated here).

## Honesty framing (read first)

- A **line-coverage floor is not a correctness proof.** It proves a line *ran*, not that any
  test would notice if its behaviour changed. `bench/coverage-floor.json` floors are
  `floor(measured) − 2` — so the epic's "floor ≥ 90" target means **measured ≥ 92%**.
- The **mutation ratchet is the real test-quality signal** (`bench/mutants-baseline.json` +
  `scripts/mutants-gate.py`): a surviving mutant is code a test executed but never asserted
  on. The gate is the **surviving COUNT** (stable), not caught% (moving denominator).
- Both are still proxies. The strongest semantic oracle is the value-level differential lane
  (sq-qcnn.2/.4 done, .5–.10 open) — this plan complements it, it does not replace it.
- Prior finding (bit us at #1250): thin public wrappers reached only *indirectly* sit ~0%
  line-covered and drag a crate below floor. **Every bead here carries the
  direct-unit-test-per-public-fn discipline** (§ Discipline below).

## Measured state (2026-07-02; live sources of truth are the two `bench/*.json` ratchet files)

### Line floors below the ≥90 target, ranked by (distance × correctness-criticality)

| crate | floor | measured (ratchet note) | criticality | note |
|---|---|---|---|---|
| sparq-canon | 77 | 79.04 | **CORE** — RDFC-1.0 canonicalization; sparq-zk commitments, sparq-vc signing. (NOT the diff-test bnode isomorphism: sq-qcnn.7 landed in `sparq-difftest::iso` on third-party `rdf-canon` directly, because the independence constraint forbids the diff-test reusing sparq's own canonicaliser.) | mutation ceiling **STALE**: committed 9, real ≈ **98** survivors (sq-52su close-note) |
| sparq-substrate | 80 | 82.49 (`numeric.rs` **71.36**) | **CORE** — shared eval substrate: XSD numeric tower, 4 join kernels, term total order | worst mutation score in the baseline: **128 survivors, 60.98% caught** |
| sparq-engine | 83 (nightly 85) | ~85 test-only / 87.31 merged-nightly | **CORE** — the SPARQL engine itself | **absent from the mutation baseline entirely** (the historical gap the epic names) |
| sparq-server | 86 | 88.14 | PROTOCOL | 2nd pass after sq-4vao |
| sparq-geo | 87 | ~89 | CORE-eval — DE-9IM topology, geof: functions | unseeded in mutants |
| sparq-reason | 87 | 89.23 | **CORE** — RDFS/OWL-RL/N3 soundness | unseeded in mutants |
| sparq-rsp | 87 | ~89 | CORE-eval — deterministic window/R2S semantics | unseeded |
| sparq-zk | 88 | ~90 | SECURITY (route **opus**) | unaudited crate (sq-qhy4); tests claim no soundness |
| sparq-mpc | 88 | ~90 | SECURITY (route **opus**) | same |
| sparq-fedclient | 89 | 91.71 | PROTOCOL | micro-raise; nearly there |
| sparq-zk-compose | 76 | ~78 | SECURITY (route **opus**) | lowest floor in the file |

### High-floor crates whose mutation score exposes the vacuous-test gap (the epic's thesis)

| crate | line floor | surviving mutants | caught% |
|---|---|---|---|
| sparq-prov | 94 | 67 | 70.09 |
| sparq-introspect | 94 | 66 | 75.37 |
| sparq-parse | 91 | committed 14 → real ≈ **32** (stale) | 72.09 |

Line coverage says these crates are fine; mutation says a third of their logic could be
silently wrong. That asymmetry is exactly why the mutation ratchet, not the line floor, is
the program's primary success metric.

### Root infrastructure blocker

Only **6 of ~29 eligible crates** are seeded in `bench/mutants-baseline.json` and the lane is
still **advisory**, because the nightly `mutants-nightly` job is **cancelled after ~5-6
crates by the workflow concurrency group** (`cancel-in-progress` racing push-to-main on the
same ref) — sq-52su close-note. Until a full nightly artifact exists, ~20 crates (engine,
core, reason, geo, rsp, shacl, …) have *no* measured test-quality signal. Fixing this is the
single highest-leverage bead in the plan (TQ1). Crate beads do **not** block on it — each can
run `cargo mutants -p <crate>` locally to enumerate its own survivors.

### Explicit non-targets (so nobody re-litigates)

- At/above target: core 90, shacl 93, serve 92, wasm 93, nlq 91, vectors 91, fedplan 94,
  prov 94, introspect 94, sim 94, algos 97, text/hdt/policy 90 (prov/introspect get
  *mutation* beads only).
- `floor: 0` measurement-artifact crates (cli, conformance, gpu, *-wasm bundles) are
  presence-gated **by design** — leave them.
- sparq-solid 89: already had its raise pass (sq-2xdr); measured ≈ 91; diminishing returns.
- JSON-LD floors are owned by sq-oy1f.36. Un-floored **CORE** reasoners sparq-reason-el /
  sparq-reason-ql are a real gap → TQ13 wires both (bundled deliberately: gate-wiring
  touches the *shared* `scripts/coverage.sh` + ratchet JSON, so one bead avoids the only
  shared-file conflict in the plan).

## Discipline (goes verbatim into every bead brief)

1. **Enumerate before writing**: run `cargo mutants -p <crate>` (shard/timeout if large) and
   `scripts/coverage.sh` first; write tests that kill *named* survivors and cover *named*
   uncovered regions — never coverage-farm.
2. **One direct unit test per public fn** — wrappers/facades reached only indirectly sit ~0%
   covered (the #1250 trap). New tests assert *values*, not just "doesn't panic" (the
   mechanical-verify gate flips an expected value to check non-vacuity).
3. **Which mutants matter** (kill these classes first; they encode SPARQL semantics):
   comparison-operator flips (`<`→`<=`) in term-ordering/compare/window-bound code;
   arithmetic sign/rounding mutations in the numeric value tower (promotion, overflow,
   `xsd:decimal` rounding, INF/NaN); `&&`↔`||` flips in guard chains (fail-closed paths);
   "replace body with `Default::default()`" survivors on public fns (pure vacuous-test
   signal). **Provably-equivalent survivors** (defensive bounds, short-circuit guards) may be
   *documented* in the baseline note and left, as sparq-reason did — never silently ignored.
4. **Measure with the crate's whole-surface features** (see the `measure()` case in
   `scripts/coverage.sh`): substrate `numeric,join,compare,rows`; fedclient
   `fedclient,fedclient-adaptive`; policy `count-enforcement`; prov `reason`. Default build
   otherwise.
5. **Re-seed both ratchets** on completion:
   `scripts/coverage-gate.py --seed target/coverage/coverage-summary.json` (floors only rise)
   and `scripts/mutants-gate.py --seed target/mutants/<crate>/outcomes.json` (ceilings only
   fall). Stale ceilings (canon, parse) must be corrected to *reality* in the same PR that
   strengthens the tests — never commit a knowingly-false baseline.
6. Tests-only PRs: no public-API changes expected, so no SKILL.md churn; keep sparq-core/
   engine lean (no new deps); README untouched unless a note is factually stale.

## The beads (prioritized; one crate per bead; each independently dispatchable)

| # | bead | crate / surface | floor → target | why | tier |
|---|---|---|---|---|---|
| TQ1 | mutants-nightly CI fix + full seed | `.github/workflows` + baseline | — | unblocks measurement for ~20 crates; engine's first-ever mutation number | sonnet (sparq-ci-infra) |
| TQ2 | substrate raise + mutant-kill | sparq-substrate | 80 → 90 | correctness core; numeric.rs 71%; 128 survivors | sonnet |
| TQ3 | engine raise + first local mutants run | sparq-engine | 83 → 90 | the flagship; zero mutation signal today | sonnet |
| TQ4 | canon raise + kill real ~98 survivors + honest re-baseline | sparq-canon | 77 → 90 | lowest CORE floor; stale ceiling; feeds bnode-isomorphism + zk/vc | sonnet |
| TQ5 | parse mutation-kill + honest re-baseline | sparq-parse | 91 (floor OK) | vacuous-test archetype: real ~32 vs committed 14 | sonnet |
| TQ6 | reason raise + local mutants seed | sparq-reason | 87 → 90 | reasoner soundness; measured 89.23, close | sonnet |
| TQ7 | geo raise | sparq-geo | 87 → 90 | DE-9IM/geof eval semantics | sonnet |
| TQ8 | rsp raise | sparq-rsp | 87 → 90 | deterministic window semantics | sonnet |
| TQ9 | server raise (2nd pass) | sparq-server | 86 → 90 | protocol conformance envelope | sonnet |
| TQ10 | prov mutation-kill | sparq-prov | 94 (floor OK) | 67 survivors @ 70% caught | sonnet |
| TQ11 | introspect mutation-kill | sparq-introspect | 94 (floor OK) | 66 survivors @ 75% caught | sonnet |
| TQ12 | fedclient micro-raise | sparq-fedclient | 89 → 90 | measured 91.71; one small test-add | haiku |
| TQ13 | wire reason-el + reason-ql into the coverage gate | scripts/coverage.sh + floors | unfloored → seeded | CORE reasoners currently outside the ratchet | sonnet |
| TQ14 | zk-compose raise | sparq-zk-compose | 76 → 85 | lowest floor overall; audit-gated crate — tests make **no soundness claim** | **opus** |
| TQ15 | zk raise | sparq-zk | 88 → 90 | commitment/encoding determinism | **opus** |
| TQ16 | mpc raise | sparq-mpc | 88 → 90 | share/reconstruct algebra | **opus** |
| TQ17 | promote mutants lane advisory → GATING | ci.yml + baseline | — | end-state of the epic; **depends on TQ1 + TQ4 + TQ5** | sonnet (sparq-ci-infra) |

Only TQ17 has dependencies. TQ2–TQ16 touch disjoint crates (plus each crate's own line in
the two ratchet JSONs — different entries, near-zero conflict surface; rebase-don't-merge if
two land close together). SECURITY-tier beads (TQ14–16) route to **Opus** per standing
policy; everything else is mechanical-once-scoped fleet work.

### Per-crate module priorities (what "correctness-critical modules first" means)

- **sparq-substrate** (`--features numeric,join,compare,rows`): `numeric.rs` first — the XSD
  numeric value tower (type promotion, exact decimal arithmetic/rounding, overflow to
  error vs saturate, canonical lexical forms, INF/NaN propagation) is at 71.36% and carries
  most of the 128 survivors. Then `join.rs` (the 4 id-tuple kernels: empty-input, dup keys,
  bind-combine ordering, LFTJ trie edges), then `compare.rs` (SPARQL term total-order
  transitivity/antisymmetry across type boundaries — property-style direct tests). `rows.rs`
  is at 100.
- **sparq-engine**: aggregate semantics (GROUP BY empty-group, COUNT vs COUNT(DISTINCT),
  AVG/SUM over mixed numerics, error-in-group), FILTER three-valued logic (error ≠ false),
  OPTIONAL/MINUS scoping edges, ORDER BY across type boundaries, planner arms (greedy-GOO vs
  the opt-in DP planner must agree — a cheap in-crate differential), and the feature-gated
  surfaces (window-functions, result-cache invalidation, txn) in *both* feature states.
- **sparq-canon**: Hash-N-Degree-Quads recursion (automorphic bnode clusters — the classic
  RDFC-1.0 hard cases), poison-graph guard boundaries (the DoS bound: exactly-at vs
  one-past), issued-id map determinism, canonical N-Quads byte-exactness, the
  `rdf12-triple-terms` feature in both states, oxrdf-bridge error paths.
- **sparq-parse**: chunk-boundary correctness in the parallel gzip/zstd paths (record split
  across chunk edge, multi-member/multi-frame concatenation byte-equality vs single-stream),
  truncated/corrupt-stream error paths. Kill the ~32 real survivors; re-seed honestly.
- **sparq-reason**: remaining `n3/mod.rs` gaps (85.17 after sq-fodw5), builtin reverse-mode +
  fail-closed arms, incremental-closure retract/re-add, `explain` proof-tree feature state;
  document any *provably-equivalent* survivors in the baseline note as before.
- **sparq-geo**: DE-9IM predicate matrix vs known WKT fixture pairs (touching-not-overlapping,
  point-on-boundary), geof: function domain errors (empty geometry, mixed CRS), R-tree
  candidate-set == brute-force on small fixtures.
- **sparq-rsp**: window advance/close boundaries (tuple exactly on the edge — where `<`→`<=`
  mutants live), RStream/IStream/DStream deltas over identical inputs, out-of-order arrival
  determinism.
- **sparq-server**: content-negotiation + error-envelope paths not covered by sq-4vao
  (json_error/sanitized_error leak-envelope), GSP edge verbs, MVCC SI/OCC conflict paths —
  direct unit tests, not just loopback integration.
- **sparq-prov / sparq-introspect**: pure survivor-kill from the seeded outcomes lists; no
  floor movement needed. Assert lineage-graph *contents* (not counts) and introspection
  *values* (not presence).
- **zk / zk-compose / mpc** (Opus): deterministic encoding/round-trip and share/reconstruct
  algebra tests only; every PR body restates that tests do **not** constitute the sq-qhy4
  soundness audit.

## Success criteria for the epic (measurable, no vanity)

1. Every non-artifact crate has a **measured** mutation ceiling (absence ≠ zero survivors).
2. The mutants lane is **gating** (TQ17), with ceilings that are *true* (no stale canon/parse).
3. All CORE-crate line floors ≥ 90 (measured ≥ 92); protocol crates ≥ 89.
4. Surviving-mutant totals for the CORE set (substrate/engine/canon/reason) trend to a
   documented floor of provably-equivalent survivors — not to an undifferentiated number.

Beads carry ids `sq-qcnn.*` under the epic; this record is the plan of record for the wave.

---

## Wave 2 (2026-07-06) — remaining floors, nightly-mutants truth, per-PR mutation subset, structural guards, Kani disposition

> [FABLE-5] Second architect pass on the same epic (one epic → **one** design record, so
> this is an update, not a new file). Verified against `origin/main` `f58a9c9f` + the live
> CI run ledger — not taken from the wave-1 text on faith.

### Corrected premises (what wave 1 got stale)

1. **"The nightly-mutants mechanism is fixed" (sq-qcnn.11 / #1378) is only PARTIALLY true.**
   The latest scheduled run (28776460517, 2026-07-06) shows **16/26 mutation shards
   succeed** (and upload artifacts), **5 are cancelled at exactly the ~2 h boundary**
   (sparq-core, sparq-engine, sparq-solid, sparq-zk-compose, sparq-vectors — a per-job
   timeout, not the old concurrency-group cancel), and **5 fail outright** (sparq-reason
   ~10 min, sparq-prov ~6 min, sparq-mpc ~57 min, sparq-server ~108 min, sparq-substrate
   ~18 min — failure logs NOT yet read; the fix bead reads the full `--log` FIRST, per the
   house CI-debugging rule). `bench/mutants-baseline.json` still holds only **6** crates.
   The epic's end-state (sq-qcnn.27 promotion) is blocked here, not on wave-1 test-writing.
2. **Two new sub-90 crates exist that wave 1 could not plan for:** the sq-6vshe.4 facade
   split created `sparq-engine-serialize` (floor 88, measured 90.95) and
   `sparq-engine-service` (floor 84, measured 86.78) after the wave-1 table was written.
3. **"Raise to 90" wave-1 beads landed floors at `floor(measured)−2`, not 90.** reason
   (88, measured 90.52), server (88, measured 90.19), engine (87, measured 89.85) all
   closed honestly below the epic's floor-≥90 bar (floor 90 ⇒ measured ≥ 92). Second
   passes are genuine remaining work, not re-litigation.
4. **Open wave-1 beads NOT duplicated here:** sq-qcnn.5–.10 (value-level diff-test lane,
   own record), sq-qcnn.23 (reason-el/ql gate wiring), sq-qcnn.27 (mutants promotion),
   sq-has2g (engine mutation-kill once seeded).

### The 2026-07-06 review-catch classes → structural guards

Today's review ledger caught, among others (verifiable instances cited): **(C1)
feature-gated test modules with no CI executor** — tests compiled but never run (#1672,
caught **twice**: missing matrix leg, then missing golden line); **(C2) gate-intent checks
placed in advisory jobs** — provably non-gating (#1679's G3 self-test moved to the hard G6
job); **(C3) jobs gating `ci-summary` without recorded probation evidence** (#1656,
gui-mock-ipc); **(C4) vacuous assertions** (the standing class the mutation ratchet exists
for; cf. the vacuous #1432 egress assertion); **(C5) proof-harness vacuity** via
`assume`/`unwind` pruning (sq-sqtk2.7; already made structural by #1536 —
`research/mechanized-proof-program.md` §5.1–§5.2 mandatory domain-coverage self-checks).
The program's job is to make C1–C4 **checked by machinery, not by reviewer vigilance**:

- **G1 — feature-gated-test execution completeness** (`scripts/check-feature-test-execution.py`,
  wired GATING in `feature-matrix.yml`): enumerate every `#[cfg(feature = …)]` /
  `#![cfg(feature = …)]`-gated test module and every `[[test]]`/`[[bench]]` target with
  `required-features`; assert each feature-set is actually enabled by ≥ 1 CI executor —
  a feature-matrix leg (`scripts/assemble-feature-matrix.py` output), a
  `scripts/coverage.sh measure()` feature-set, or an explicit allowlist entry with a
  reason (device-gated GPU, wasm-pack-only). Fail-closed on unmapped. Static +
  deterministic ⇒ gating from day one, fixture-tested in both directions (a synthetic
  ungated feature-test must go RED). Makes the #1672 class structurally impossible.
- **G2 — advisory-job registry + gate-intent placement** (`.github/advisory-registry.json`
  + `scripts/check-advisory-registry.py`, wired in `docs-quality.yml`): every job whose
  name matches the ci-summary `\b(advisory|informational)\b` exclusion regex must carry a
  registry entry `{owner_bead, promotion_criteria, registered}`; and no step invoking a
  gate-classified script (`scripts/*gate*.py`, G-numbered self-tests) may sit inside an
  advisory-named job without an explicit waiver. Generalises the E2E gating-policy ledger
  (#1657) repo-wide. Makes the #1679 + #1656 classes visible-by-diff instead of
  visible-by-catch.
- **C4 (vacuous tests)** is covered by the mutation machinery — the missing piece is the
  **per-PR subset** (M2 below): nightly-only mutation signal arrives days after the
  vacuous test merges; `--in-diff` gives the signal at PR time on exactly the changed code.

### Per-PR mutation subset (M2) — design decision

The full ratchet stays **nightly** (cargo-mutants rebuilds + reruns the suite once per
mutant; a full run is hours per big crate — never per-PR). Per-PR we add a **dedicated
workflow** (`.github/workflows/mutants-diff.yml` + `scripts/mutants-diff.sh`) running
`cargo mutants --in-diff` over the PR's merge-base diff, bounded (mutant cap + per-run
timeout + `--baseline skip` where sound), **ADVISORY at introduction** and registered in
G2's registry with explicit promotion criteria (stable across N observed PRs, median
runtime within budget, a survivors-allowlist mechanism designed). Rationale for
advisory-first rather than gating: surviving mutants on new code include
provably-equivalent ones (see the sparq-prov/sparq-mpc baseline notes), so an unallowlisted
hard gate would force either assertion-gaming or blanket overrides — the PR-time value is
*review signal* ("your new tests never notice this mutant"), promoted only once the
allowlist idiom exists. A dedicated workflow file keeps it off the required `ci-summary`
path and file-disjoint from sq-qcnn.27's `ci.yml` promotion edit.

### Kani / #1487 disposition (inventory only — owned by sq-sqtk2, no bead created here)

Draft PR #1487 (WAC/ACP decision-core proof harnesses, `crates/sparq-solid`) is **not
landable as-is** and is honestly labelled so in its own body: no complete
`cargo kani -p sparq-solid` run exists (zero harnesses completed in ~50 min under
contention). It belongs to epic sq-sqtk2 (bead sq-sqtk2.1, blocked by sq-sqtk2.7), not to
this epic. What it takes to land, per the PR body + sq-sqtk2.7 notes: (a) the in-flight
re-scoping — FlattenCompat elimination, symbolic-string shrinking, `unwind` 24→40 (which
also fixes a real vacuity: `unwind(24)` silently `assume`-pruned the 32–39-byte
PUBLIC/AUTHENTICATED/ANY_* principal paths, so the deny-wins/fail-closed harnesses were
vacuous for exactly the security-relevant identities); (b) a complete tee'd kani log
quoting every `VERIFICATION:- SUCCESSFUL` line (likely needs the dedicated/uncontended
runner sq-sqtk2.7 recommends); (c) the deny-wins mutation RED→GREEN evidence; (d)
domain-coverage self-checks per `research/mechanized-proof-program.md` §5.1–§5.2 —
mandatory since #1536. Program touchpoint: C5 is this epic's vacuous-test class applied to
proofs and is **already codified there** — nothing to duplicate. Consequence for this
wave: the `sparq-solid` coverage raise (floor 89) stays **deferred** (wave-1 non-target +
live file-collision with the #1487 branch).

### Wave-2 beads (disjoint; created under the epic 2026-07-06)

| # | bead | surface (files) | tier | invariant | acceptance |
|---|---|---|---|---|---|
| W2-M0 | sq-qcnn.28 — fix the 10 non-green nightly mutants shards | `.github/workflows/ci.yml` (mutants job only) | sonnet | lane stays advisory + off the required path; no gating job weakened | next scheduled/dispatched nightly: 26/26 mutation shards `success` + artifacts complete |
| W2-M1 | sq-qcnn.29 — seed every complete-artifact unseeded crate | `bench/mutants-baseline.json` | sonnet | no knowingly-false ceiling (entries only from COMPLETE rollups; absence ≠ zero) | `scripts/mutants-gate.py --check` green; new entries byte-match their artifact rollups |
| W2-M2 | sq-qcnn.30 — per-PR `--in-diff` mutation lane | `.github/workflows/mutants-diff.yml` + `scripts/mutants-diff.sh` (new files) | sonnet | advisory + bounded; required `ci-summary` path untouched | forced lane failure does not block a PR; diff-only mutant set demonstrated on a probe PR |
| W2-G1 | sq-qcnn.31 — feature-gated-test execution check | `scripts/check-feature-test-execution.py` + fixtures + `feature-matrix.yml` | sonnet | every feature-gated test module/target has ≥ 1 CI executor (fail-closed) | check RED on synthetic ungated fixture, GREEN on tree; gating leg wired |
| W2-G2 | sq-qcnn.32 — advisory-registry + gate-placement check | `scripts/check-advisory-registry.py` + `.github/advisory-registry.json` + `docs-quality.yml` | sonnet | no unregistered advisory job; no gate-script in an advisory job w/o waiver | check RED on both fixture classes, GREEN on tree with seeded registry |
| W2-R1 | sq-qcnn.33 — sparq-engine-serialize 88→90 | `crates/sparq-engine-serialize/**` + its floor entry | haiku | floors only rise; direct value-asserting tests (no vacuity) | `cargo llvm-cov -p sparq-engine-serialize --features serialize-rdf,streaming-serialization` ≥ 92; `coverage-gate.py --check` at floor 90 |
| W2-R2 | sq-qcnn.34 — sparq-engine-service 84→90 | `crates/sparq-engine-service/**` + its floor entry | sonnet | same + SSRF egress tests stay fail-closed | `cargo llvm-cov -p sparq-engine-service --features service` ≥ 92; floor 90 |
| W2-R3 | sq-qcnn.35 — sparq-engine 87→90 (exec.rs 86.99, explain.rs 85.55 first) | `crates/sparq-engine/**` + its floor entry | sonnet | result-equivalence: tests pin exact SPARQL semantics values | `cargo llvm-cov -p sparq-engine` ≥ 92; floor 90 |
| W2-R4 | sq-qcnn.36 — sparq-reason 88→90 2nd pass (n3/mod.rs 88.92) | `crates/sparq-reason/**` + its floor entry | sonnet | reasoner answer-soundness pins (exact derived triples) | `cargo llvm-cov -p sparq-reason` ≥ 92; floor 90 |
| W2-R5 | sq-qcnn.37 — sparq-server 88→90 2nd pass | `crates/sparq-server/**` + its floor entry | sonnet | error-envelope/protocol tests assert exact bodies/status | `cargo llvm-cov -p sparq-server` ≥ 92; floor 90 |
| W2-R6 | sq-qcnn.38 — sparq-zk-compose 85→90 (**route OPUS, maintainer-arm**) | `crates/sparq-zk-compose/**` + its floor entry | opus | determinism/fail-closed tests only; **NO ZK soundness/privacy claim** (external audit pending, sq-qhy4) | `cargo llvm-cov -p sparq-zk-compose` ≥ 92; floor 90; privacy-claims gate green |

**Dependency edges (ordering that is real, plus registry-adjacency sequencing):**
sq-qcnn.27 ← {W2-M0, W2-M1} (promotion needs a fully-green, fully-seeded lane);
sq-has2g ← W2-M1 (engine mutation-kill starts from a seeded ceiling);
W2-M2 ← W2-G2 (the new advisory lane registers itself in G2's registry file);
W2-R2 ← W2-R1 and W2-R3 ← W2-R2 (their `coverage-floor.json` keys are textually
**adjacent** — sequenced to honour no-two-in-flight-beads-on-adjacent-lines);
W2-R4 ← sq-qcnn.23 (sq-qcnn.23 inserts `sparq-reason-el`/`-ql` entries adjacent to
`sparq-reason`'s).

**Shared-registry caveat (the only intentional disjointness exception, wave-1 precedent):**
every W2-R bead edits exactly its own crate entry in `bench/coverage-floor.json`. Raise
beads must **NOT** touch `bench/mutants-baseline.json` (owned by W2-M1 this wave — their
crates' mutation ceilings arrive via the nightly seed, and mutation-kill passes get their
own later beads once survivor counts are real, mirroring sq-has2g).

### Wave-2 success criteria (delta over wave 1's)

1. A scheduled nightly exists with **26/26** mutation shards green; `mutants-baseline.json`
   covers every non-excluded crate (absence ≠ zero survivors stays true until then).
2. All sub-90 core-logic floors raised to 90 except the two **documented** deferrals:
   `sparq-solid` (#1487 collision) and the `floor: 0` measurement-artifact crates.
3. G1 + G2 are gating, fixture-tested, and the current tree passes both — C1/C2/C3 become
   diff-visible.
4. The per-PR `--in-diff` lane runs on real PRs as registered-advisory with recorded
   promotion criteria.
5. sq-qcnn.27 (promotion to gating) is unblocked by measurement, not by optimism.
