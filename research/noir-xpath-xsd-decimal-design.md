<!-- [OPUS-5] sq-n5e7p (#3357): decision record for the `xs:decimal` gap in the externalized
noir_XPath face repo. DESIGN-FOR-REVIEW only — no production code, and none is possible here:
the module lives in sparq-org/noir_XPath, not in this repo. -->
# Bounded 18-digit `xs:decimal` fixed-point arithmetic for `noir_XPath`

Maintainer-review decision record for bead **`sq-n5e7p`** (issue **#3357**) — the `xs:decimal`
deferral inherited from the deleted `IMPLEMENTATION_PLAN.md` ("Deferred due to fixed-point
arithmetic complexity in ZK circuits"), surfaced by the `sq-u3u15` repo-hygiene sweep.

Sits under [`zk-correctness-and-proof-program.md`](zk-correctness-and-proof-program.md), which
owns the `sq-3x7dl.*` XPath findings — in particular `sq-3x7dl.4`, whose fix introduced the
`numeric_divide_int_as_double` **documented approximation** this record exists to remove, and
`sq-3x7dl.5`, which fixed `get_common_type(float, decimal)`. It does not restate or contradict
them.

---

## 0. Honesty framing (read first)

- **Bounded precision, NOT arbitrary precision.** What is decided here is an **18-digit,
  comptime-fixed-scale subset** of `xs:decimal`, not the unbounded value space the datatype
  defines: every value satisfies `mag < 10^18` at a scale `S <= 18`, giving `S` fraction digits
  and `18 - S` integer digits (§2.1). That is exactly the "at least 18 decimal digits" a
  *minimally conforming* processor owes (XSD 1.1 Part 2 §3.3.4), and F&O 3.1 §4.2 licenses an
  implementation-defined precision limit — so the subset is conformant. It does **not** close
  the general arbitrary-precision gap, and no wording here, in the parent program record, or in
  any implementing PR may say it does. The range is **implementation-defined and fail-closed**:
  an input, an aligned operand, or a result outside it makes a constrained assert fire (§2.3,
  §3) — it is never silently rounded, truncated, or wrapped. Choosing a larger budget later
  means changing the representation, not a constant.
- **This record decides; it does not implement.** Per `sq-5reoy` (#1599) the `zk/xpath` tree was
  externalized and **removed from this repo** — `git ls-files zk/xpath` returns only the
  differential harness (`zk/xpath/differential`, `zk/xpath/scripts`,
  `zk/xpath/tests/differential_oracle`). `xpath/src/numeric_types.nr` is in
  [`sparq-org/noir_XPath`](https://github.com/sparq-org/noir_XPath). The implementation
  therefore lands in the **face repo** (source of truth), behind an `XPATH_TAG` bump here.
  Nothing in this record can be, or is, code.
- **The issue's status pointers are stale.** It cites `zk/xpath/SPARQL_COVERAGE.md` and
  `ARCHITECTURE.md` as authoritative; neither exists in this repo — both moved with the tree.
  Read the face repo for those, and this record for the decision.
- **Grounding.** The semantics below are from **XPath/XQuery F&O 3.1 §4.2** (arithmetic
  operators on numeric values, including the `err:FOAR0001` divide-by-zero rule for
  `xs:decimal`/`xs:integer` operands and the implementation-defined-precision note) and **XSD
  1.1 Part 2 §3.3.4** (`xs:decimal`; a minimally conforming processor must support at least 18
  decimal digits). The *representation* choice is grounded in two things that are **in this
  repo and were read**: sparq's own exact fixed-point decimal
  (`crates/sparq-substrate/src/numeric.rs`, `Dec` / `Num::Dec`) and sparq's own in-circuit
  fixed-scale decimal precedent (`zk/compose/compose_core/src/filter_signed.nr`,
  `filter_decimal_check<ID, FD>`, bead `sq-1q9h`).
- **[verify-at-impl].** This box has no copy of the face repo and no warm `~/.nargo` cache.
  Every claim about `noir_XPath`'s *current* contents — that `numeric_types.nr` carries only a
  `NumericType::decimal()` enum tag at `type_id=1` with no value type, the exact
  `numeric_divide_int_as_double` signature, the `get_common_type` table, `u128` availability on
  the face repo's pinned `nargo` — is taken from #3357's own description, from
  [`skills/zk-query-proofs/SKILL.md`](../skills/zk-query-proofs/SKILL.md), and from the
  committed differential golden's `use xpath::{…}` import list. Each is marked
  **[verify-at-impl]** and must be re-checked against the face repo before a line is written.
  No decision here rests on unread face-repo code.
- **No security claim.** `noir_XPath` is a value-semantics library. The ZK estate remains
  **research-grade and NOT externally audited**; external accredited-cryptographer sign-off is
  still pending (`sq-qhy4`). Nothing here asserts any circuit is sound, proven, or audited.
  §3 records the one *soundness-shaped* consequence of the representation choice — a
  non-canonical zero — as an **obligation**, not an achievement.
- **Reachability.** `zk/compose` does **not** consume `noir_XPath`
  ([`zk-audit-readiness-dossier.md`](zk-audit-readiness-dossier.md) §1.3), so nothing decided
  here is on a sparq proof path today. This is a library-conformance gap, not a live defect.

---

## 1. What is actually missing

`op:numeric-divide` on two `xs:integer` operands yields an `xs:decimal` quotient (F&O 3.1 §4.2,
`op:numeric-divide` — `7 div 2 = 3.5`, distinct from `op:numeric-integer-divide`/`idiv`'s
truncating `3`; cite the operator, not a subsection number — `skills/zk-query-proofs/SKILL.md`
already carries a differing subsection for the same rule **[verify-at-impl]**).
`sq-3x7dl.4` de-aliased the
two operations but, having no decimal value type to land in, implemented the divide as
`numeric_divide_int_as_double`: `i64 → f64` promotion plus correctly-rounded IEEE 754 division,
**documented as an approximation**. That is the whole of the gap:

| Surface | Today | With a decimal type |
| --- | --- | --- |
| `op:numeric-divide(int, int)` | binary64 approximation | exact-or-declared-precision decimal |
| `NumericType::decimal()` (`type_id=1`) | enum tag with no inhabitant **[verify-at-impl]** | a real value type |
| `int` ⊕ `decimal` promotion | cannot occur (no decimal values) | must resolve to `decimal` |
| `op:divide-dayTimeDuration-by-dayTimeDuration` (`sq-3x7dl.9`) | truncates to integer | decimal ratio |

`sq-3x7dl.9` is the second consumer and is the reason this is worth doing beyond the divide:
it is blocked on exactly this type.

---

## 2. DECISION — comptime-fixed scale, sign-and-magnitude, 18-digit budget

### 2.1 The representation

```rust
// xpath/src/decimal.nr  (proposed)
pub struct XsdDecimal<let S: u32> {
    neg: bool, // sign; canonical: mag == 0 ==> neg == false
    mag: u64,  // |value| * 10^S
}
```

The value denoted is `(-1)^neg * mag * 10^-S`. Three invariants, all **constructor-enforced**
(§3):

1. `S <= 18` — a `std::static_assert`, so it is a compile-time property of the circuit family.
2. `mag < 10^18` — the declared precision. `10^18 < 2^63 < u64::MAX`, so an 18-digit magnitude
   fits `u64` with headroom, and this is exactly the "at least 18 decimal digits" a minimally
   conforming processor owes per XSD 1.1 Part 2 §3.3.4.
3. `mag == 0 ==> !neg` — `xs:decimal` has no negative zero (unlike `xs:double`), so the
   encoding must not admit two zeros.

`S` fraction digits leaves `18 - S` integer digits. **`S` is a circuit-family parameter the
host fixes and binds**, not a value the prover chooses — the same posture
`filter_decimal_check<ID, FD>` already takes in `zk/compose/compose_core/src/filter_signed.nr`,
and the same posture as the `filter_f64` digit-count parameter. This record deliberately
declares **no single default `S`**: the right value is a property of the query's literals
(§2.4), and baking one in would silently round somebody's inputs.

### 2.2 Why not the two alternatives

- **Variable scale** (sparq's own `Dec { mant: i128, scale: u32 }`). Rejected for the circuit.
  Aligning two operands needs `10^(s_max - s_i)` for a *runtime* `s_i`, which in-circuit is
  either a lookup over the whole scale range or a bounded square-and-multiply loop — paid on
  **every** operation, including the cheap ones. It also needs a signed 128-bit mantissa, which
  Noir's signed integer tower does not reach (`i64` is the widest) **[verify-at-impl]**, so it
  would have to be emulated. The host has no such constraint, which is exactly why the *oracle*
  keeps variable scale and the *circuit* does not — and why §4.3 compares **values**, not
  lexicals.
- **One global fixed scale** (e.g. `S = 18` for everything). Rejected: an 18-digit budget spent
  entirely on the fraction leaves **zero** integer digits, so `1 div 3` would be representable
  and `10 div 3` would not. Making `S` a family parameter costs nothing (it is comptime) and
  buys the whole range.

### 2.3 The operations

All at a common comptime `S`; alignment between two `XsdDecimal<S>` is a no-op. Between
different scales `S_lo < S_hi` the multiplier `10^delta` (`delta = S_hi - S_lo`) is comptime but
the **product is not**: with `mag < 10^18` and `delta` reaching 18, `mag * 10^delta` reaches
`10^36`, so a `u64` multiply **wraps** and a post-hoc `mag < 10^18` assert then passes on a
wrong value. Alignment therefore widens first: `let wide: u128 = mag as u128 * pow10(delta)`,
`assert(wide < 10^18)`, and only then narrow to `u64`. (Asserting `mag < 10^(18 - delta)`
*before* the `u64` multiply is equivalent; the `u128` form is preferred because it is the same
shape as `mul`/`div` below.) The rule this instantiates binds every product in this section:
**range-check in a wider type or before multiplying — never on a value that may already have
wrapped.**

| Op | Construction | Overflow / error posture |
| --- | --- | --- |
| `eq` / `lt` / `le` / `gt` / `ge` / `ne` | the signed order over `(neg, mag)`: a negative is `<` any non-negative; among negatives the larger magnitude is the smaller value | total; no error |
| `neg()` | `neg = !neg & (mag != 0)` | preserves invariant 3 |
| `checked_add` | same sign: `mag_a + mag_b`; opposite: larger magnitude minus smaller, taking its sign; then renormalize per invariant 3 | both operands `< 10^18`, so the `u64` sum is `< 2·10^18 < u64::MAX` and **cannot wrap before** the `mag < 10^18` assert fires — the assert is meaningful, not post-hoc |
| `checked_sub` | `checked_add(a, neg(b))` | as above |
| `checked_mul` | `p: u128 = (mag_a as u128) * (mag_b as u128)` — widened *before* the multiply, per the rule above (`< 10^36 < 2^127`), then rescale by `10^S`: `q = p / 10^S`, `r = p % 10^S`, `q += (2r >= 10^S)` | assert `q < 10^18` |
| `checked_div` | **`assert(mag_b != 0)`** first; `n: u128 = (mag_a as u128) * pow10(S)` — widened before the multiply (`< 10^36`), `q = n / mag_b`, `r = n % mag_b`, `q += (2r >= mag_b)`; sign is the xor | zero divisor is `err:FOAR0001` — **fail-closed, no Infinity escape**, matching the posture `numeric_divide_int_as_double` already takes for decimal operands |
| `round_to_int(Ceil \| Floor \| HalfUp)` | mirrors `Dec::round_to_int`'s euclidean form on the signed value | total |
| `from_i64` | magnitude first, in arithmetic that **never negates `i64::MIN`**: `m: u64 = if n < 0 { ((-(n + 1)) as u64) + 1 } else { n as u64 }` — for `n = i64::MIN`, `-(n + 1)` is `i64::MAX`, so every signed step stays inside `i64` and the `+ 1` happens in `u64`, where `2^63` is in range; then widen — `wide: u128 = (m as u128) * pow10(S)` | `assert(wide < 10^18)` before narrowing to `u64`. `\|i64::MIN\|` is **not** representable as an `i64`, so a direct `-n`/`abs()` overflows *ahead of* any range assert; the `n + 1` detour and the widen are both load-bearing, not stylistic. `m * 10^S` likewise must not be a `u64` multiply. Uses `i64`, `u64`, `u128` only — **no signed `i128`**, so this row does not depend on the §2.2 signed-tower question either way |
| `numeric_divide_int_as_decimal` | `from_i64(a).checked_div(from_i64(b))` | the exact replacement for the `sq-3x7dl.4` approximation |

Every intermediate — including the two conversions above — closes inside `u128`. The widest are
`mag_a · mag_b` (`mul`), `mag_a · 10^S` (`div`), `mag · 10^delta` (alignment), each bounded by
`10^36`, and `|i64::MIN| · 10^18 < 9.3·10^36` (`from_i64`); all are well under
`u128::MAX ≈ 3.4·10^38`. `2r` is bounded by `2·10^18`. Every **signed** value stays inside `i64`
— the widening above is `u128`, an *unsigned* type — so §2.2's rejection of variable scale (which
needs a signed 128-bit mantissa) and the constructions here are consistent: nothing in this
section requires a signed integer wider than `i64`. The only type-availability question left open
is `u128` on the pinned `nargo` **[verify-at-impl]**.

### 2.4 Rounding — half away from zero, on results only

**Rounding rule: half away from zero** (`q += (2r >= den)` on the *magnitude*). Two independent
reasons, and they agree:

- It is what sparq's own oracle does. `Dec::checked_div` computes on unsigned magnitudes and
  rounds `num % den * 2 >= den` (`crates/sparq-substrate/src/numeric.rs`), i.e. half away from
  zero on the value. Choosing anything else would manufacture an oracle divergence for no gain
  — F&O §4.2 leaves the precision-limited case implementation-defined, so both are conformant
  and only one is checkable against the harness.
- It is exactly CPython `decimal.ROUND_HALF_UP` ("ties going away from zero"), which makes the
  out-of-repo third reference in §4.2 a one-liner rather than a re-derivation.

**Rounding applies to RESULTS ONLY. Inputs are exact-or-fail.** A lexical or `i64` whose value
needs more than `S` fraction digits, or more than `18 - S` integer digits, must make the
constructor **assert**, never silently round. This is the `sq-jxh15` principle applied to the
input side: refusing can only make a witness unsatisfiable, whereas silently rounding an input
makes the *false* proposition "this circuit evaluated your literal" witnessable. Result
rounding is different in kind — F&O explicitly licenses it, and the rounded value is the
declared answer, not a substituted input.

The practical consequence, which callers must be told: **`S` must be at least the largest scale
appearing in the query's decimal literals**, or the constructor fails closed.

---

## 3. The one soundness-shaped obligation: a constrained constructor

`XsdDecimal` must be built by a **constrained** constructor, never by a bare struct literal
reachable from witness data. If a prover can supply `{neg: true, mag: 0}`, there are two
encodings of zero, and `eq` becomes prover-choosable on an input the verifier believes is
pinned. That is the same failure class as `sq-3x7dl.1` (the unconstrained IEEE-754 canonical
decode) and as the `sq-3x7dl.5` `i8`-wrap defect: a *wrong value is witnessable*.

**Obligation.** The constructor asserts, on every path that can receive witness data:

- `mag < 10^18` (a range constraint, not a comment);
- `mag != 0 | !neg` (no negative zero);

and every arithmetic result is renormalized through the same path, so no operation can *emit* a
non-canonical value either.

**Acceptance test (must exist, must be a `should_fail`).** A negative test that constructs
`{neg: true, mag: 0}` and one that constructs `mag = 10^18` from witness data **fails**
construction, while the canonical forms still pass — the same shape as the `sq-3x7dl.1`
acceptance test. A green positive-only suite does not discharge this.

Two further `should_fail` rows pin the widening rules of §2.3, and both are chosen so that an
unwidened implementation **wraps** rather than merely exceeding the bound — a `u64` intermediate
lands back in range, does *not* assert, and so reds the `should_fail` test, which is the point:

- aligning a scale-`S_lo` value to `S_hi` where the unbounded `mag · 10^delta` exceeds **both**
  `10^18` and `u64::MAX` (e.g. `mag = 10^17 - 1` with `delta = 18`);
- `from_i64(i64::MIN)` at any `S >= 1`, which a signed `abs()`/`-n` overflows before reaching
  the range assert.

**Gate cost is an obligation, not a claim.** `u128` division lowers to euclidean division — the
expensive primitive per [`noir-optimization-paper.md`](noir-optimization-paper.md) — so `mul`
and `div` are the cost centers while `add`/`sub`/`compare` are cheap. The implementing PR
**records the measured `bb gates` delta**; this record states no number and none may be
inferred from it.

---

## 4. Verification plan — the oracle half, which IS in this repo

`zk/xpath/differential` (PROOF **M1**, `sq-3x7dl.14.2`) is the in-repo half and needs no new
machinery, only new corpus. See [`zk/xpath/differential/README.md`](../zk/xpath/differential/README.md).

### 4.1 Primary oracle — sparq's own exact decimal

sparq's evaluator already has the exact type: `Num::Dec(Dec { mant: i128, scale: u32 })`, with
`checked_add`/`checked_sub`/`checked_mul` exact and `checked_div` scale-minimal-or-half-up-at-18.
So the corpus is generated the same way every existing row is — read back from a real
`BIND(<expr> AS ?out)` over a one-row graph, no hand-written expected values.

Proposed generated test functions (each must register a fault site — §4.4):

- `differential_oracle_decimal_arith` — add/sub/mul across sign combinations, scale-crossing
  operands, and the `10^18` boundary.
- `differential_oracle_decimal_divide` — terminating quotients (`1 div 8`), non-terminating
  (`1 div 3`, `100 div 7`), exact ties at scale `S` (the half-away-from-zero rule is only
  tested by a row that *is* a tie), and sign combinations.
- `differential_oracle_decimal_compare` — the signed order, including the two-zeros case
  (`0.00` vs `-0` rejected at construction) and equal-value-different-scale pairs.
- `differential_oracle_numeric_divide_int_as_decimal` — the `sq-3x7dl.4` replacement, held
  against `7 div 2 = 3.5` and the `idiv` de-aliasing control that already exists.

### 4.2 Third, out-of-repo reference — CPython `decimal`

The harness's existing discipline is that two in-repo references agreeing is weaker evidence
than it looks, so `fn:substring` is gated by CPython string slicing. The decimal analogue is
direct: `decimal.Decimal` with `decimal.ROUND_HALF_UP` and
`quantize(Decimal(1).scaleb(-S))` states the same rounding rule independently. Every emitted
decimal row is cross-checked against it, a disagreement **fails** generation, and — following
the existing pattern — if `python3` is absent the check **reports loudly that it verified
nothing** rather than passing quietly.

### 4.3 The one expected divergence — scale, not value

sparq's `Dec` is variable-scale, so `1 div 2` gives it `Dec { mant: 5, scale: 1 }` → the
lexical `"0.5"`. A fixed-scale-`S` circuit answers `0.5` at scale `S` → `"0.500000000"` at
`S = 9`. **The values agree; the lexicals do not.** Therefore:

- decimal assertions compare **values at a common scale**, never lexicals;
- a quotient that terminates at some `s <= 18` but needs more than `S` fraction digits is
  rounded by the circuit and *not* by the oracle. Such a row is still emitted **live**, with the
  rounded expected value and a **`ROUNDED-AT-SCALE`** label at the row and in the file header —
  a third label alongside the existing `SPEC-REFERENCE`. It is never silently dropped and never
  commented out (a commented assertion cannot fail and verifies nothing).

That labelling rule is the honest reading: such a row asserts `noir_XPath == round_S(sparq)`,
not `noir_XPath == sparq`.

### 4.4 Non-vacuity

Unchanged and non-negotiable: every new generated `#[test]` registers a fault site, appears in
`--list-fault-sites`, and gets its own `--inject-fault-in` variant whose `nargo test` **must
fail**. A new test function that registers no site makes generation panic — that guard already
exists and must not be relaxed for these.

---

## 5. Sequencing — three steps, in this order

The harness rows **cannot land first**: the generated `lib.nr` `use xpath::{…}` imports must
compile against the pinned tag, and the differential lane pins `XPATH_TAG: "v0.2.0"`
(`zk/xpath/scripts/run_differential_harness.sh`, `.github/workflows/xpath-differential.yml`)
while the face repo's latest release is `v0.3.0`. Committing decimal rows before the type
exists reds the lane.

1. **Face repo** — land `decimal.nr` per §2/§3 with its own `nargo test` suite including the
   `should_fail` canonicality tests; cut a release. **No caller changes.**
2. **This repo** — bump `XPATH_TAG`, extend the generator with the §4 corpus, regenerate the
   committed golden, confirm the per-test fault-injection loop is green-then-red.
3. **Face repo, separately** — re-point `op:numeric-divide` (and `sq-3x7dl.9`'s duration ratio)
   from the double approximation to the decimal path.

**Step 3 changes answers and must not be folded into step 1.** `1 div 3` is
`0.3333333333333333` as a binary64 today and `0.333333333` as a decimal at `S = 9`; the
committed golden pins the current double answers, and `int` ⊕ `decimal` promotion — inert today
because the tag has no inhabitant — starts resolving to `decimal` the moment one exists, which
can change results on any mixed expression **[verify-at-impl]**. Keeping `numeric_divide_int_as_double`
in place and unchanged through steps 1–2 keeps each step's diff reviewable and each regeneration
of the golden attributable.

---

## 6. What this record does not decide

- **`S` for any particular caller.** §2.1 makes it a family parameter on purpose.
- **A decimal `fn:format-number` / lexical serializer.** Not needed by `op:numeric-divide`; the
  compose tree's `filter_decimal_check` already rebuilds a fixed-shape lexical for *binding*,
  which is a different job.
- **Anything about `zk/compose`.** It does not consume `noir_XPath` (§0), so no compose circuit,
  gate count, or verifier property is touched by any of the above.
- **Any soundness or privacy property.** Unchanged: research-grade, not externally audited,
  `sq-qhy4` pending.
