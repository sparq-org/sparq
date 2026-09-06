# Design — ZK correctness + optimization + proof-of-correctness program (`sparq_ieee754` & `noir_XPath`)

<!-- [OPUS-4.8] sq-5reoy (#1599): the in-tree `zk/ieee754` and `zk/xpath` Noir trees were externalized to the `sparq-org/noir_IEEE754` (v0.10.0) and `sparq-org/noir_XPath` (v0.2.0) face repos and REMOVED from this repo; `zk/compose` now consumes the released `sparq_ieee754` as a pinned Nargo git dependency, and §3's `nargo test` CI wiring (`sq-3x7dl.13`) now lives in each face repo's own CI. Any `zk/xpath/…` / `zk/ieee754/…` path below is a HISTORICAL in-tree reference — the live source is the corresponding face repo. -->

<!-- [FABLE-5] Fable-tier synthesis of three upstream audits (ieee754 correctness/opt/CI,
xpath correctness/opt/CI, sparql_noir/Lean prior-art). DESIGN + decomposition only — NO
production code in this PR. 🤖 SPARQ agent. -->

> 🤖 **SPARQ agent** — design record for @jeswr's review. DESIGN-FOR-REVIEW only.

**Status:** DESIGN / verification + proof-**feasibility**. This record consolidates three
upstream audits and lays out a remediation, CI-gating, and proof program. It is **not** a
claim that any circuit is already proven, nor that any MPC/ZK security property is achieved.

**Umbrella epic:** `sq-3x7dl`. **Proof sub-epic:** `sq-3x7dl.14`. **MPC/ZK-extension spike:**
`sq-3x7dl.16`. **External-audit gate (unchanged, master):** `sq-qhy4`.

**Scope of the two libraries under audit:**

- `zk/ieee754` — `sparq_ieee754`, IEEE 754 binary16/32/64/128 for Noir (comptime width
  param; a `u64` "work" kernel for f16/f32/f64 and a `u128`/`Field` "wide" kernel for f128).
- `zk/xpath` — `noir_XPath`, XPath 2.0 Functions & Operators for SPARQL 1.1 (`zk/xpath/xpath`,
  tests in `xpath_unit_tests` + the generated `test_packages`).

Both are consumed by the ZK query-proof estate (`crates/sparq-zk`, `crates/sparq-zk-compose`,
`zk/compose`) as the trusted value-semantics lemma base: float arithmetic/comparison and
XSD/XPath scalar functions inside the proof circuits reduce to these kernels.

**Honesty preamble.** The upstream audits were performed *statically* — `nargo`/`bb` are not
installed on the audit box, so findings are from constraint analysis and spec cross-reference,
not execution. Every remediation bead therefore ships its own executable acceptance test, and
the CI track (§3) exists precisely so those tests actually run. No performance number is
hard-coded anywhere in this record; every optimization is gated on a measured `bb gates`
delta (§2).

---

## 0. One-line recommendations

1. **Fix the one soundness hole first** (`sq-3x7dl.1`): `sparq_ieee754`'s public `new(bits)`
   decode is **under-constrained** — a malicious prover can decode a bit pattern into
   non-canonical fields denoting a *different* real number and prove false float statements.
   This is the highest-value fix and the only true circuit-soundness defect found.
2. **Wire `nargo test` into CI for `zk/ieee754` + `zk/xpath`** (`sq-3x7dl.13`, **P1**,
   near-term must-do): today neither library's Noir tests run in the monorepo at all, so both
   correctness *and* soundness regressions merge green.
3. **Remediate the XPath silent-wrong-value bugs** (`sq-3x7dl.4`–`.9`): truncating integer
   divide, i8-truncated mixed comparisons, `substring` start < 1, NUL-vs-capacity length
   confusion, pre-1970 dateTime, `min`/`max` over empty.
4. **Treat "proof" as a phased escalation** (`sq-3x7dl.14`): a trusted reference oracle +
   differential harness in CI is the cheap high-value foundation; mechanized proof is
   reserved for the highest-value/highest-risk primitives and is honestly expensive.
5. **MPC/ZK extension is a design spike only** (`sq-3x7dl.16`) — and formal proof
   **complements, never replaces**, the external cryptographer audit `sq-qhy4`.

---

## 1. CORRECTNESS — consolidated findings, invariants, acceptance tests

Each finding maps to exactly one bead on exactly one source file (disjoint, so the fleet can
work them in parallel without merge conflicts). Tiers: **fable** = soundness-subtle circuit
fix; **sonnet** = mechanical-but-careful value semantics; **haiku** = docs / dead code / tiny.
No unaudited crypto is labelled "sound" anywhere.

### 1.1 `sparq_ieee754`

| Bead | Sev | File | Tier | Defect (short) |
|------|-----|------|------|----------------|
| `sq-3x7dl.1` | **HIGH** | `src/codegen.nr` | **fable** | `new(bits)` decode under-constrained; non-injective re-encode |
| `sq-3x7dl.2` | low | `src/ops/kernels.nr` (+`codegen.nr`) | sonnet | no SPARQL/XPath round-half-toward-+∞ mode |
| `sq-3x7dl.3` | low | `README.md`, `ct.nr` | haiku | stale "Known gaps"; dead/broken `ct.nr` |

**`sq-3x7dl.1` — the soundness hole (fable).**
The raw-bits decode is the primary public constructor and the ZK trust boundary. The only
in-circuit constraint on the prover-chosen `(sign, exponent, mantissa)` witnesses is
`assert_eq(decoded.bits(), bits)`, where `decoded` comes from an `unsafe { decode_unconstrained }`
block. The struct field *types* are wider than the real IEEE fields (mantissa `u16/u32/u64/u128`
vs 10/23/52/112 real bits; exponent `u8/u16` vs 5/11/15), and nothing asserts
`mantissa < 2^mant_size` or `exponent < 2^exp_size`. Because
`bits() = sign·2^(W-1) + exponent·2^mant_size + mantissa`, the surplus mantissa bits overlap the
exponent lane, so the packing is **not injective**: a malicious prover can substitute
non-canonical fields that re-encode to the same `bits` yet denote a different real number.

- *Concrete counterexample (f32):* `bits = 0x3F800001` (= 1 + 2⁻²³). Canonical decode
  `{sign:0, exp:127, mantissa:1}`. Malicious decode `{sign:0, exp:126, mantissa:0x800001}`:
  `126·2²³ + 0x800001 = 127·2²³ + 1 = 0x3F800001` (identical), `mantissa` fits the `u32` field,
  `exp` fits `u8`, and all `u32` overflow checks pass. Downstream `classify`/`significand`
  then read it as 1 + 2⁻²⁴, not the true 1 + 2⁻²³. The same `{exp−1, mantissa+2^mant_size}`
  attack applies to every width.
- **Invariant:** `new(bits).bits() == bits` **and** the `(sign, exponent, mantissa)`
  decomposition is the *unique* canonical IEEE decode (`exponent < 2^EXP_SIZE`,
  `mantissa < 2^MANT_SIZE`) for every width.
- **Fix:** in the constrained side of `new()`, add explicit width bounds mirroring the
  `assert_max_bit_size` discipline already used throughout `kernels.nr`, e.g.
  `(self.mantissa as Field).assert_max_bit_size::<MANT_SIZE>()` and the exponent analogue
  (comptime-substituted per width). Adds a small, fixed number of range constraints per
  `new()`. This is a soundness fix, not a perf negotiation.
- **Acceptance test:** a negative/forge test that feeds the non-canonical witness
  `{exp−1, mantissa+2^mant_size}` for a fixed bit pattern **fails** the constrained `new()`
  (`should_fail`), per width, while the canonical decode still passes; existing `lib.nr` tests
  stay green; PR records the `bb gates` delta (a small increase is expected and acceptable).

**`sq-3x7dl.2` — SPARQL/XPath ROUND mode (sonnet).** The round-to-integral family implements
IEEE `roundToIntegral{TowardNegative,TowardPositive,TowardZero,TiesToEven}` but has no
round-half-toward-+∞, which is exactly what SPARQL 1.1 `ROUND()` and XPath 2.0 `fn:round`
require. `round_ties_even(2.5)=2`, `(0.5)=0`, `(-0.5)=-0` (correct IEEE); SPARQL wants
`ROUND(2.5)=3`, `ROUND(0.5)=1`, `ROUND(-0.5)=0`.
**Invariant:** `ROUND(n+0.5) == n+1` for all representable `n`; the four IEEE modes byte-unchanged.
**Fix:** add a fifth mode; do **not** substitute `floor(x+0.5)` (the intermediate FP add is
wrong near the boundary). Depends on `sq-3x7dl.1` (shares `codegen.nr`).

> **Adjacent, distinct:** `sq-3x7dl.2` is the *round-to-integral* family. Directed rounding
> for the *arithmetic* ops (`rndu`/`rndd`/`rndz`/`rna` on add/sub/mul/div) is a separate,
> kernel-level gap tracked as `sq-xs0pa` (#3140); its decision record is
> [`noir-ieee754-directed-rounding-design.md`](noir-ieee754-directed-rounding-design.md).
> Both touch the round-and-pack step, so sequence them rather than running them in parallel.
> Wider still, `sq-i6f4l` (#3155) evaluates the whole set of API items dropped from the
> published surface in the Float-API migration — `abs`, that directed-rounding arithmetic,
> and `Field`↔float — in
> [`noir-ieee754-dropped-api-evaluation.md`](noir-ieee754-dropped-api-evaluation.md); it
> **routes** the directed-rounding item to the record above rather than re-deciding it, and
> notes that `abs` is a sign-lane transform that does *not* touch round-and-pack, so it does
> not need that sequencing.

**`sq-3x7dl.3` — docs + dead code (haiku).** README "Known gaps" lists implemented ops
(comparisons, sqrt, round-to-integral, casts) as "Not yet implemented"; refresh it. Delete
`ct.nr` — an orphan (Noir compiles only `src/`; unreferenced) *and* internally broken (a stale
`sizing.nr` duplicate whose `exponent_size` loop never terminates). The genuine constant-time
rationale already lives in the `codegen.nr` comment.

### 1.2 `noir_XPath`

All findings are **silent wrong values** (not honest circuit errors), which is the dangerous
class in a proof system: the circuit accepts and "proves" a wrong answer. One bead per file.

| Bead | Sev | File | Tier | Defect (short) |
|------|-----|------|------|----------------|
| `sq-3x7dl.4` | **HIGH** | `src/numeric.nr` | sonnet | `op:numeric-divide` truncates + aliased to `idiv`; `fn:number` whitespace |
| `sq-3x7dl.5` | **HIGH** | `src/numeric_types.nr` | **fable** | mixed int↔float/double compare truncates i64→**i8** |
| `sq-3x7dl.6` | HIGH/MED | `src/string.nr` | sonnet | `substring` start < 1 length; NUL-vs-capacity length; byte-vs-codepoint STRLEN |
| `sq-3x7dl.7` | MED | `src/datetime.nr` | sonnet | pre-1970 dateTime corrupted by `u64` casts |
| `sq-3x7dl.8` | MED | `src/sequence.nr` | sonnet | `min`/`max` over empty asserts (unprovable) |
| `sq-3x7dl.9` | low | `src/duration.nr` | haiku | duration/duration truncates to integer |

**`sq-3x7dl.5` is tiered fable** because a truncated mixed comparison produces a *false boolean*
that a proof circuit will accept — e.g. `compare_int_double_eq(256, 0.0)` returns **true**
(256 `as i8` == 0), and `compare_int_double_eq(1000, 1000.0)` returns **false** (1000 `as i8`
= −24). These back SPARQL numeric `FILTER` comparisons, so a wrong boolean here is a
soundness-relevant wrong answer. **Invariant:** for all `i64 x` and `f64/f32 y`,
`compare_int_*(x,y)` equals the mathematically-correct XPath ordering/equality (NaN unordered,
+0 == −0). **Fix:** an exact `i64→f64` conversion (i64 exact in f64 up to 2⁵³; f32 via IEEE
round-to-nearest-even), used in *all* mixed comparisons. **Acceptance:** the should-be-false /
should-be-true cases the current i8 code gets wrong become passing tests.

`sq-3x7dl.4` (divide returns `xs:decimal`/`double`, de-alias `idiv`) and `sq-3x7dl.5` both need
an exact `i64→f64` conversion; they live in different files so each can carry a local helper
(no shared-file conflict), and a later cleanup may unify. `sq-3x7dl.7` is a mechanical refactor
that mirrors the **already-correct** `date.nr` (signed `i32` + `floor_div_i64`). Full
per-finding invariants, fixes, and acceptance tests are in each bead body.

> **Downstream of `sq-3x7dl.4`:** its fix landed `numeric_divide_int_as_double`, a *documented
> binary64 approximation*, because the module has no `xs:decimal` value type to land the
> quotient in — `numeric_types.nr` carries only the `NumericType::decimal()` enum tag. Building
> that type — a **bounded 18-digit, comptime-fixed-scale subset**, not arbitrary precision, with
> anything outside the declared range failing closed — and thereby unblocking `sq-3x7dl.9`'s
> duration ratio, which truncates for the same reason, is `sq-n5e7p` (#3357); its record is
> [`noir-xpath-xsd-decimal-design.md`](noir-xpath-xsd-decimal-design.md). Note the sequencing it
> fixes: re-pointing `op:numeric-divide` at the decimal path **changes answers** the committed
> `sq-3x7dl.14.2` golden currently pins, so it is a separate step from adding the type.

---

## 2. OPTIMIZATION — measured, soundness-preserving only

The repo's own history warns that broad hot-kernel rewrites have regressed gates
(`noir-optimisation` skill; `bench/SPIKES.md`), and the audit's dominant cost center — the
per-shift `pow2` verifiers in `kernels.nr` — is explicitly **do-not-weaken** (a shared-table
variant risks re-introducing an under-constrained shift, the same failure class as
`sq-3x7dl.1`). So the optimization beads are limited to algebraic-identity cleanups and one
cast restructuring, and **every one carries the same hard gate**:

> **HARD GATE (mandatory on every optimization bead):** the change **MUST NOT reduce
> soundness**. Prove the circuit is **still fully constrained** — include a negative/differential
> test that an *under-constrained* version of the rewrite would **fail** — and confirm the gate
> delta is neutral-or-lower with `bb gates -s ultra_honk` **before** adoption (`nargo info`
> alone is misleading). If `bb gates` shows a regression, do not adopt.

| Bead | File | Tier | Opportunity (all identity-preserving) |
|------|------|------|----------------------------------------|
| `sq-3x7dl.10` | `ieee754/src/ops/kernels.nr` | sonnet | `jam_low_bit(q,s) == q \| (s as int)`; CSE the duplicate exponent-equality in `compare_parts` |
| `sq-3x7dl.11` | `xpath/src/numeric_types.nr` | sonnet | single signed barrel-shift in `cast_float/double_to_integer` (one variable shift removed) |
| `sq-3x7dl.12` | `xpath/src/string.nr` | sonnet | combined `index_of` primitive for `contains`/`substring_before`/`after`; single-diff string compare |

Each optimization bead **depends on** its correctness counterpart on the same file
(`.10`→`.2`, `.11`→`.5`, `.12`→`.6`) so they serialize rather than conflict. The value of
`.10` is mainly DRY/readability (expected neutral); `.11`/`.12` may save real gates *if the
codegen actually shares the work* — which is exactly why the gate is measured, not asserted.

---

## 3. CI GATING (near-term must-do) — `nargo test` for `zk/ieee754` + `zk/xpath`

**This is the highest-priority near-term item** (`sq-3x7dl.13`, **P1**) and the maintainer
asked for it explicitly.

### 3.1 The gap (grounded in the real workflow)

`.github/workflows/zk-toolchain.yml` is the *only* ZK lane. It installs the pinned Noir/`bb`
toolchain but its load-bearing step is:

```sh
cargo test -p sparq-zk-compose -p sparq-zk -- --ignored --test-threads=1
```

i.e. the **Rust** forge/anchor suite. Consequences confirmed by both audits:

- The `sparq_ieee754` `#[test]`s (in `lib.nr`) and the `noir_XPath` inline `#[test]`s (in
  `xpath/src`) **never execute in CI**. A correctness/soundness regression in `kernels.nr` or
  `string.nr` that still *compiles* merges green — including the `new()` under-constraint
  (`sq-3x7dl.1`), which passes every existing check.
- `zk/xpath` has library-local workflows under `zk/xpath/.github/workflows/` (`nargo test`,
  `nargo fmt`, gate-count); `zk/ieee754` has no `.github/` directory and no library-local
  workflows. GitHub Actions only runs workflows from the repo-root `.github/workflows/`, so
  `zk/xpath`'s library-local workflows are **inert** in the monorepo — and neither library's
  Noir tests currently run in CI, which `sq-3x7dl.13` addresses.
- The `zk-toolchain` path filter *does* include `zk/**`, so an xpath/ieee754 change surfaces a
  green `zk forge + anchor suite` check — a **false** sense of coverage: that job never compiles
  or tests the changed Noir.

### 3.2 Plan (reuse the existing structure)

Extend `zk-toolchain.yml` (or add a sibling lane) that:

1. **Reuses the already-pinned** `noirup`/`bbup` install steps and the **`merge_group`
   changed-files gate** pattern already in that workflow (do not float toolchain versions; the
   two-level supply-chain pin stays intact).
2. Runs `nargo test` in `zk/ieee754` and `zk/xpath/xpath`, gated on the **same** zk paths filter
   (`crates/sparq-zk/**`, `crates/sparq-zk-compose/**`, `zk/**`, the workflow file) plus the
   merge-group detect-changes gate — so it surfaces a check on zk-touching PRs and fast-passes
   elsewhere, matching the existing lane's posture and never hanging `ci-summary`.
3. **Scopes the invocation to the real test targets.** `nargo test --workspace` over `zk/xpath`
   is currently **RED**: the audit found 426/543 generated qt3 test chunks call `assert(false)`
   stubs and 7 packages are bare `assert(false)` placeholders. So the initial lane runs the
   `sparq_ieee754` `lib.nr` tests and the `noir_XPath` `xpath/src` + `xpath_unit_tests` real
   tests, and **excludes** the stub `test_packages` until `sq-3x7dl.15` regenerates them against
   the real functions. Otherwise the lane is red on day one.
4. **Best-effort fold-ins** (if cheap): the `ieee754` exact-rational vector oracle
   (`scripts/test_generated_vectors.sh`) and the public-API encapsulation guard
   (`scripts/test_public_api.sh` + `lint_private_function_usage.py`). The gate-regression
   benchmark (`benchmark_float_ops.py` / `compare_float_benchmarks.py`) is a separate follow-up
   (needs `bb` + a committed baseline).
5. Carries the comment: *"until `zk/ieee754` + `zk/xpath` are split into their own repos, this
   lane is their standing `nargo test` gate."*

**Acceptance:** on a PR that intentionally breaks an `ieee754`/`xpath` `#[test]` the lane goes
RED and `ci-summary` blocks; a clean zk change is green; a non-zk change fast-passes; the stub
`test_packages` are excluded so the lane is green on real code today.

**Follow-up bead `sq-3x7dl.15`** regenerates the qt3 corpus against the real functions (so the
conformance oracle validates production code and would *catch* the `sq-3x7dl.4`–`.7` bugs
pre-fix); once it lands, the lane drops its stub exclusion.

---

## 4. PROOF-OF-CORRECTNESS PROGRAM (phased, feasibility-honest)

Sub-epic `sq-3x7dl.14`. Drawn from the `sparql_noir` prior-art (which, load-bearing correction,
contains **no Lean** — the "Lean correspondence" is an *aspiration* the maintainer's own prior
project evaluated via Lampe and explicitly **deferred** as not-yet-cost-justified, shipping prose
`SAFETY PROOF` doc-comments + differential testing instead).

**Why these libraries are a good proof target.** Every kernel is **pure, deterministic,
fixed-width, and (mostly) straight-line / data-oblivious**, with no runtime table lookups (2^k
is built by product chains / bit recomposition). The design is systematically
**hint-and-verify**: every `unconstrained` hint is paired with an in-circuit verifier, so the
formal obligation reduces to *"each verifier uniquely pins its hint."* That is exactly where the
`new()` finding bites — its verifier does **not** uniquely pin its output — which makes it both
the first correctness fix *and* a natural first proof target (an equivalence proof of `new()`
against canonical IEEE decode would correctly **fail** today).

### 4.1 Milestones and their trusted computing base (TCB)

Every tier below — even full Lean — still trusts the Noir→ACIR→Barretenberg lowering; **source-
level proof never eliminates the backend**. That is stated honestly at each tier.

| Milestone | What it buys | Effort | Confidence | TCB (honest) |
|-----------|--------------|--------|------------|--------------|
| **M1** differential oracle in CI (**beaded**) | bit-exact verification vs a trusted oracle on a *sample* | small | high | oracle + sample coverage + lowering |
| **M2** exhaustive binary32 unary ops | *complete* reference-semantics coverage for unary ops | medium | high | oracle + lowering (no sampling gap for exhausted ops) |
| **M3** mechanized relation-adequacy | shrinks TCB to "this relation pins the answer" | medium | medium | relation adequacy + range-check completeness + lowering |
| **M4** SMT/BMC on bit sub-primitives | all-inputs guarantee where tractable (QF_BV/QF_FP) | large | medium | SMT model fidelity + solver + lowering |
| **M5** Lampe/NAVe Lean equivalence | all-inputs machine-checked; smallest *runtime* TCB | research | speculative | Lean kernel + Lampe's fidelity to `nargo` + lowering |

**M1 — the recommended first milestone (beaded now):** a trusted reference oracle + differential
harness wired into CI.

- **`sq-3x7dl.14.1` (ieee754):** native Rust hardware `f32`/`f64` **is** the IEEE-754
  round-to-nearest-even reference, so the harness asserts **bit-equality** (not approximation)
  for `+,−,*,/,sqrt,compare,casts`, covering ±0, ±inf, NaN payloads, subnormal boundary,
  ties-to-even, over/underflow, exponent-difference sweeps. f16/f128 via a soft-float crate
  (MPFR/`rug` or a berkeley-softfloat binding). This is a genuine improvement over the prior
  art, which used MPFR *approximation* — for the 32/64-bit default-rounding paths sparq's oracle
  is **exact**.
- **`sq-3x7dl.14.2` (xpath):** sparq's own trusted Rust SPARQL/XSD scalar evaluator
  (Oxigraph-parity) as the oracle for STRLEN/CONTAINS/STRSTARTS/STRENDS/SUBSTR/numeric-casts/
  comparisons/ROUND over a unicode-aware corpus. It doubles as the regression oracle for the
  `sq-3x7dl.4`–`.7` fixes.

M1 is **verification, not proof** — that is stated in each harness README along with its TCB.
It is the cheap, strong, honest baseline everything else builds on.

### 4.2 Later milestones (described, not fully beaded)

- **M2 — exhaustive binary32 unary ops.** binary32 is only 2³² inputs, so every *unary* value op
  (abs, neg, round/ceil/floor/trunc, sqrt, f32→f64, i32↔f32, isNaN, EBV, unary compares) can be
  checked **exhaustively** against the hardware-f32 oracle in minutes-to-hours, sharded on EC2 —
  a *complete* guarantee for the reference semantics. Honest caveats: exhausting the *oracle*
  proves the spec's value semantics; binding the actual **circuit** needs pushing 2³² through
  `acvm` witness-gen (far slower) — practical compromise: exhaust oracle-vs-Rust-reimpl at native
  speed, then let the in-circuit relation (M3) tie the circuit to that algorithm. binary32
  *binary* ops (2⁶⁴) and all binary64 are not exhaustible → fall back to M1 sampling.
- **M3 — mechanized relation-adequacy.** Keep the hint-and-verify pattern but upgrade the prose
  `SAFETY PROOF` to (i) adversarial `should_fail_with` negatives **and** (ii) a machine-checked
  (SMT or Lean — the relation is tiny) argument that the asserted relation *uniquely determines*
  the correct IEEE/XPath result. Shrinks the security-relevant TCB from "the whole op is correct"
  to "this one relation pins the answer."
- **M4 — SMT/BMC on bit sub-primitives.** The bit-manipulation core (clz, mantissa/exponent
  extraction, guard/round/sticky, shift-normalize) is pure fixed-width logic, decidable in QF_BV,
  with the rounding decision checkable against QF_FP — all-inputs where tractable. Targets
  *sub-primitives*, not the field-encoded end-to-end op (which is beyond QF_FP). The Noir→SMT
  extraction/maintenance is the real cost; the field-encoded ACIR stays trusted.
- **M5 — Lampe/NAVe Lean equivalence.** Extract each primitive into Lean 4 via Reilabs' Lampe,
  hand-write a Lean spec (IEEE-754-2019 value semantics / XPath-XSD function semantics; the
  `sparql-formal-semantics` skill already carries the PAG algebra), and prove a correspondence
  theorem over all inputs. Largest payoff, smallest *runtime* TCB (Lean kernel + Lampe fidelity),
  but expert-months per non-trivial op and young tooling. **Reserve for a handful of
  highest-value/highest-risk primitives, only after M1–M3.**

### 4.3 Feasibility verdict

**Yes — proving `ieee754`/`xpath` correct is tractable, incrementally, but "proof" means
different strengths at each tier and the honest framing matters.** The pure/fixed-width/
hint-and-verify shape is unusually favorable. M1 (verification) is cheap and high-value **now**;
M2 gives *complete* reference coverage for the unary binary32 ops (a real all-inputs guarantee
exactly where it is affordable); M3/M4 give machine-checked guarantees for the security-relevant
cores (the verifier relations and the trickiest bit-twiddling); full M5 Lean equivalence is
expert-months per op and stays a targeted research bet, not the default. **No tier removes the
Noir→ACIR→Barretenberg lowering from the TCB** — that boundary is only closed by the external
audit `sq-qhy4`, not by source-level proof.

---

## 5. MPC / ZK-SPARQL extension (design/feasibility only — **no implementation beaded**)

Design spike `sq-3x7dl.16` (fable, `depends-on sq-qhy4`). Once the `ieee754`/`xpath`
differential-then-mechanized method (§4) is established, it extends — with *different* target
properties — to the crypto estate. The audits cover **single-prover ZK value semantics**; the
extension covers the **protocol/security** layer, where the proof obligations are categorically
harder and where the external audit is load-bearing.

**Properties worth proving, per crate:**

- **`sparq-zk` / `sparq-zk-compose` (single-prover ZK query proofs).**
  - *Soundness* — a forged proof/manifest is rejected. This is the estate's core claim; the
    `forge_pubinput_*` suite is its empirical form. The **highest-value first target** is the
    verifier's `reconstruct_public_inputs` serialization: a field-order / endianness / arity slip
    re-opens the whole estate, and it is exactly a hint-and-verify *relation-adequacy* obligation
    — the **same** method as `ieee754` `new()` (§4). Differential-testable now; mechanizable at M3.
  - *Completeness* — an honest proof verifies (differential/positive controls).
  - *Zero-knowledge* — the proof leaks nothing beyond the public statement. This is a
    **simulation** argument, **not** reachable by the value-level differential method; it needs a
    cryptographic proof (simulator construction) and is squarely audit territory.
- **`sparq-mpc` (honest-majority, semi-honest Shamir).**
  - *Semi-honest security / correctness vs the ideal functionality* — a simulation-based argument
    that the real protocol view is indistinguishable from the ideal. The tractable *first slice*
    is **per-gadget correctness** (share arithmetic, MAC-share authentication, the hidden-value
    join, robust reconstruction) checked against a Rust **ideal-functionality oracle** by the same
    differential method; the *security* (indistinguishability) half remains a cryptographic proof.
    Scope stays explicit: honest-majority, semi-honest — **not** malicious, **not**
    dishonest-majority (per the crate README / `SECURITY.md`).
- **`sparq-fedplan-mpc` (opt-in planner↔MPC seam).**
  - *Leakage-envelope soundness* — a property-based check that the declared leakage envelope
    **upper-bounds** the actual disclosure of the disclosed-vs-hidden operator partition, plus the
    fail-closed dual-ratification. Property-testable now; the formal statement is a
    non-interference-style argument.

**Phased sketch (mirrors §4):** verification-first (differential vs ideal-functionality/reference
oracles; property tests for the leakage envelope) → relation-adequacy mechanization for the
security-relevant relations (verifier public-input reconstruction; MPC gadget correctness) →
simulation / Lean for the genuinely cryptographic properties (ZK-ness, MPC indistinguishability),
reserved for the highest-value targets.

**Hard honesty constraint (bakes into the spike verdict):** formal proof **complements but does
not replace** the external accredited-cryptographer audit **`sq-qhy4`**. Correctness-vs-ideal and
soundness-of-a-relation are *value/logic* obligations the method can attack; **zero-knowledge and
MPC indistinguishability are cryptographic obligations** that a differential/relation proof does
not discharge. **Never state that proof alone clears a production ZK/MPC security claim, and never
label unaudited crypto "sound."** The spike produces a written verdict + TCB table only — no
implementation and no implementation beads.

---

## 6. Bead tree (created by this record)

Umbrella epic **`sq-3x7dl`**. All children are disjoint by file (or dependency-ordered where they
share one), tiered, and each carries an executable acceptance test in its body.

- **Correctness (§1):** `sq-3x7dl.1` (fable, ieee754 `codegen.nr`), `.2` (sonnet, `kernels.nr`),
  `.3` (haiku, `README`+`ct.nr`); `.4` (sonnet, `numeric.nr`), `.5` (fable, `numeric_types.nr`),
  `.6` (sonnet, `string.nr`), `.7` (sonnet, `datetime.nr`), `.8` (sonnet, `sequence.nr`),
  `.9` (haiku, `duration.nr`).
- **Optimization (§2, hard gate):** `.10` (ieee754 `kernels.nr`), `.11` (xpath `numeric_types.nr`),
  `.12` (xpath `string.nr`).
- **CI gating (§3, P1):** `.13`; corpus follow-up `.15`.
- **Proof program (§4):** sub-epic `.14` with M1 beads `.14.1` (ieee754) + `.14.2` (xpath);
  M2–M5 described here, not beaded.
- **MPC/ZK extension (§5):** design spike `.16` (`depends-on sq-qhy4`).

**Standing caveat.** This record is verification + proof-feasibility. Nothing here asserts that
any circuit is proven or that any MPC/ZK privacy/soundness property is achieved. `sq-qhy4` remains
the gate for any production security claim.
