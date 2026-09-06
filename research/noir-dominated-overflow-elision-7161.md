# Dominated overflow-check elision (noir#7161) — design record (sq-jthy1)

**Bead:** `sq-jthy1` (epic `sq-uuvac`, `noir-optimization-program.md` §7 row 6, §10.3 *"`ssa/opt/checked_to_unchecked.rs`;
elide overflow checks dominated by a later range check (#7161); Brillig failure-point semantics in scope"*) ·
**Status:** **design + measurement complete; NOT implemented, no upstream PR proposed.** One HEAD prerequisite must land
first (§5) · **Author:** SPARQ agent 🤖 [OPUS-5] · **Date:** 2026-07-28

Analysed and measured against `noir-lang/noir` @ `a352e7152a28afd5e15b363ddf32ba88df28f312` (2026-07-28, workspace version
`1.0.0-beta.25`), in a throwaway clone outside this repo. Every line citation is at that commit. Companion records:
`noir-optimization-program.md` (§3.2 SSA pass map, §4.2 landmine list, §6 upstream protocol, §10 fleet spec),
`noir-not-canonicalization-ifelse-merging.md` (the measured-null-result precedent + the toolchain recipe),
`noir-acir-expression-growth-4629.md` (the expression-growth interaction in §7).

## 0. What this record is, and what it is not

This is a **design-for-review record**, not an implementation. What was actually done, and what bounds every claim:

| capability | available | consequence |
|---|---|---|
| noir source at HEAD + full history | yes (shallow clone) | source analysis; all citations pinned to `a352e715` |
| rustc `1.89.0` (the workspace pin) | yes — installed into a writable `RUSTUP_HOME` (§10) | `cargo test -p noirc_evaluator` builds and runs |
| the **SSA interpreter** (upstream's executable spec) | yes | §3 is *measured* ACIR-vs-Brillig behaviour, not inferred |
| `nargo` built from an instrumented tree | yes (dev profile, optimized) | §6 is a *measured* candidate-frequency count over 509 packages |
| `bb` | **no** | **no gate counts anywhere in this record** |
| the patched optimization itself | **not written** | **no ACIR-opcode delta, no before/after circuit numbers** |
| upstream issue #7161 body + state | read (2026-07-28) | §1 quotes it; no comment was posted |

Nothing here has been proposed upstream. Per `noir-optimization-program.md` §6/§10.2 any PR is draft-first with the
generative-AI disclosure and @jeswr's author review before maintainer review.

## 1. The ask

Upstream **[#7161](https://github.com/noir-lang/noir/issues/7161)** — *"Investigate removing overflow checks for
intermediate arithmetic where an overflow would guarantee a future overflow"*, TomAFrench, type `Enhancement`,
**OPEN**, project status `📋 Backlog`, no discussion comments. Body, verbatim:

> Consider the program
>
> ```text
> acir(inline) fn main f0 {
>         b0(v0: u32):
>           v2 = add v0, u32 1
>           v3 = add v1, u32 1
>           return v3
>         }
> ```
>
> I'm fairly certain (check this) that if we ran ACIRgen on this we'd end up with two separate range checks for `v2`
> and `v3`. However it's impossible for `v2` to fail its overflow check without `v3` also doing so. We can then skip
> the overflow check for `v2` and rely on `v3`'s.
>
> We could then add a pass which removes these unnecessary overflow checks to make more of the circuit's arithmetic
> unchecked.
>
> Note that this likely would change how certain tests fail so we may want to have a brillig overflow check inserted
> here. To be determined by whoever works on this.

Two corrections to the framing this record starts from:

1. **The snippet has a typo:** `v3 = add v1, u32 1` references an undefined `v1`; the intended chain is
   `v3 = add v2, u32 1`. Everything below assumes the intended chain.
2. **"Dominated by a later check" is the wrong relation** (the bead title inherits it). What the argument needs is that
   the later check is *guaranteed to execute* whenever the earlier one would have — post-dominance, not dominance. For
   ACIR this is free: `flatten_cfg`'s postcondition is *"each constrained function should now consist of only one
   block where the terminator instruction is always a return"* (`flatten_cfg.rs:15-16`), and `checked_to_unchecked`
   runs long after flattening (`ssa/mod.rs:315` vs `:396`). Within one block, "later in program order" **is**
   guaranteed execution. No dominator tree is required for the ACIR case.

The issue's core claim ("it's impossible for `v2` to fail without `v3` also failing") is **confirmed** — see §3 — but it
is confirmed *only* under a set of side conditions that the issue does not state and that are individually easy to
violate (§4). The Brillig caveat is not a "may want to" nicety; it is a hard correctness boundary (§3 row 4).

## 2. Why this works at all: the ACIR/Brillig split

An unsigned checked `add`/`sub`/`mul` becomes, at ACIR-gen, the arithmetic plus **one range constraint on the result at
the type's bit width**, carrying the `"attempt to … with overflow"` assertion payload
(`acir/mod.rs:798-821` → `acir_context.rs:1126-1160`). An `unchecked_*` op emits the arithmetic and nothing else, and —
this is the load-bearing fact — **ACIR has no wrapping**: the value simply extends in the field.

Upstream states this in four places, all at HEAD:

| citation | statement |
|---|---|
| `ssa/interpreter/mod.rs:1336-1352` | *"Unchecked arithmetic: Brillig wraps to the bit size; ACIR extends in the field."* |
| `ssa/interpreter/value.rs:504-506` | an out-of-range field *"only arises from unreduced ACIR arithmetic"* |
| `ssa/ir/dfg.rs:586-617` | `get_value_max_num_bits` reports a **conservative** bound for ACIR unchecked `add`/`sub`/`mul` — *"its result may exceed the operands' type width until a later range check or truncation brings it back … so callers never mistake that width for a range proof"* |
| `ssa/ir/dfg/simplify/binary.rs:332-335` | the tree-wide invariant: *"The only values that can hold a magnitude outside their type's range are results of unchecked `add`/`sub`/`mul`."* |

So the elision has a natural, already-supported representation: **flip the checked op to `unchecked_*`; never delete a
`RangeCheck` instruction.** The flip inherits every existing consumer's conservatism for free — `get_value_max_num_bits`
(dfg.rs:594-617), `operand_max_num_bits` (dfg.rs:635-652), and `can_simplify_arithmetic_identity`
(simplify/binary.rs:336-359), which already refuses to fold `x + 0 → x` when `x` is an unchecked result. It also keeps
the invariant sentence quoted above literally true after the transform. A design that instead removed the range
constraint at ACIR-gen, or deleted a `RangeCheck`, would have to re-derive all of that.

One consequence of the flip is *not* neutral and must be stated: a checked op `requires_acir_gen_predicate`
(`instruction.rs:931-942`) and an unchecked one does not, and `range_constrain_var` **returns `predicate · result`**,
not `result` (`acir_context.rs:1144-1159`). So flipping also drops a multiplication by the side-effects predicate,
changing the value seen downstream when the predicate is false (0 before, the raw field value after). HEAD's
`checked_to_unchecked` already does this on every flip, so the "values computed under a disabled predicate are not
observed" contract is upstream's, not something this optimization introduces — but §4 C4 keeps the new pass inside the
same predicate region so it never *widens* that assumption.

## 3. Measured semantics (SSA interpreter, ACIR and Brillig)

Run against the interpreter at HEAD (`ssa.interpret`), one u32 argument `v0 = u32::MAX`. Each row is the whole program;
"elided" means the first `add` was hand-written as `unchecked_add`, i.e. exactly what the proposed pass would produce.

| # | program (chain, then variation) | runtime | result |
|---|---|---|---|
| 1 | `v2 = add v0,1; v3 = add v2,1` (baseline) | ACIR | `Err(Overflow …)` reported at **`add v0, u32 1`** |
| 2 | `v2 = unchecked_add v0,1; v3 = add v2,1` | ACIR | `Err(Overflow …)` reported at **`add v2, u32 1`**, with `v2` shown as the out-of-range field `(4294967296)` |
| 3 | same as 1 | **Brillig** | `Err(Overflow …)` at the first `add` |
| 4 | same as 2 | **Brillig** | **`Ok(1)`** — the overflow silently disappears |
| 5 | `v2 = unchecked_add v0,1; v3 = sub v2,1` | ACIR | **`Ok(4294967295)`** — a non-monotone step masks the overflow (baseline errors) |
| 6 | `v2 = unchecked_add v0,1; v3 = mul v2,0` | ACIR | **`Ok(0)`** — multiplying by zero masks it |
| 7 | `v2 = unchecked_add v0,1; v3 = truncate v2 to 32 bits; v4 = add v3,1` | ACIR | **`Ok(1)`** — an intervening truncate masks it |

Rows 1–2 are the issue's claim, confirmed: the failure is **preserved**, and only its **attribution moves** to the later
operation. Row 4 is the issue's Brillig note, made concrete: in Brillig the same flip converts a trap into a wrong
answer, so **the pass must be ACIR-only** (`if !self.runtime().is_acir() { return; }`, exactly as
`check_u128_mul_overflow.rs:37-39` does). Note that this makes the issue's *"we may want to have a brillig overflow
check inserted"* unnecessary: leaving Brillig untouched preserves Brillig behaviour by construction, at the cost of the
ACIR and Brillig runtimes reporting a failure at different source spans for the same input (§4 C6).

Rows 5–7 are the reason the optimization is not a two-line pattern match: each is a plausible chain shape in which the
later check does **not** cover the earlier one.

## 4. The elision rule and its side conditions

**Rule (v1).** In an **ACIR** function, a checked unsigned **`add`** `r = add(a, b)` may be flipped to `unchecked_add`
when some later instruction in the same block *proves* that `r`'s own range check is implied. C0–C6 must all hold.

- **C0 — the elided operation is a checked unsigned `add`.** `sub` and `mul` are excluded **as the elided target**, and
  `sub` is excluded for a soundness reason, not a rent one. An unsigned checked `sub` also emits a result range check
  (§2), but ACIR has no wrapping, so an underflowing `a - b` (`a < b`) escapes **downward**: its field representative is
  `p - (b - a)`, just *below the modulus*, not just above `2^bit_size`. Every downstream argument in this section models
  the escaped result as a small non-negative integer that stays **ordered below** its descendant — C2's "monotone
  non-decreasing", C5's `bit_size + ⌈log2(k+1)⌉` bound, and the proof's `r ≤ d` step all assume upward escape only, and
  all are false modulo `p` for an underflow. Concretely, one later monotone `add` of any `w > b - a` takes
  `p - (b - a) + w` past the modulus and back down to the small value `w - (b - a)`, which then passes its own range
  check: the original failure is **lost outright**, not merely re-attributed to a later op. That is strictly worse than
  the rows 5–7 masking shapes, and it is the opposite of the rows 1–2 result the issue asks for. This is an analytic
  argument from the §2 citations, not a measured row — no `sub` target was interpreted or counted (§6 counts only
  `add`/`mul` ops), which is why v1 excludes the case rather than bounding it. `mul` is excluded as a target on the
  weaker C5 magnitude grounds (§8 step 3).
- **C1 — a covering check exists downstream.** Either (a) `r` is an operand of a later **checked** unsigned op of the
  same bit width (whose implicit range check at ACIR-gen is the cover), or (b) `r` reaches a later
  `Instruction::RangeCheck { value, max_bit_size }` with `max_bit_size ≤ bit_size(r)`. Variant (b) **never fires on the
  corpus** (§6) — recommend scoping v1 to (a) alone. Under (a) the covering op is itself the **last step** of the chain
  and is therefore subject to C2: rows 5–6 of §3 are exactly the case where the covering op is a `sub`, or a `mul` by
  zero, and so covers nothing.
- **C2 — every step from `r` to the covering check is monotone non-decreasing over the integers-in-the-field.** An
  `add` qualifies when the *other* operand is itself known to fit its type — a constant, a block parameter, or a
  checked result, per the invariant quoted in §2 — because only then is its field representative a small non-negative
  integer that cannot wrap the sum; an operand that is itself an `unchecked_*` result may have escaped its range and
  must be accounted for under C5 instead. A `mul` qualifies only by a constant `≥ 1`. Rows 5–6 of §3 are the
  counterexamples: a `sub` step, or a `mul` by a possibly-zero value, lets the chain come back into range after the
  intermediate escaped it.
- **C3 — no reducing operation intervenes.** `truncate` (§3 row 7), `and`/`xor`/`or`, `mod`, `div`, `shr` all
  re-bound the value and destroy the cover. A `cast` is transparent (it keeps the bit pattern; narrowing casts are
  preceded by an explicit `truncate` — `ssa/interpreter/mod.rs:625-628`), so casts are permitted in the chain **only**
  once §5 is fixed, because the max-bits helper currently loses the extended magnitude through exactly that edge.
- **C4 — same predicate region.** The covering check must be active whenever the elided check would have been. After
  flattening, a checked op's range check is applied to `predicate · result` (`acir_context.rs:1144-1149`) and a
  `RangeCheck` is rewritten onto `value · predicate` by flattening itself (`flatten_cfg.rs:1472-1479`). The cheap
  sufficient condition is: **no `EnableSideEffectsIf` instruction between the elided op and its cover.** Anything
  weaker (reasoning about predicate implication) is a soundness argument this record does not make.
- **C5 — field capacity.** The chain must not be able to reach the modulus, or the "overflow only pushes the value up"
  argument wraps and the cover is lost. This is not hypothetical at `u128`: upstream's own
  `test_programs/execution_failure/u128_multiplication_overflow` exists because *"using bn254 field elements wraps the
  maximum field element value, resulting in [a value] which is less than u128::MAX. We want this to be caught as an
  overflow"*, and `check_u128_mul_overflow.rs:1-8` documents the same capacity arithmetic. v1 should track a
  per-chain magnitude bound (`bit_size + ⌈log2(k+1)⌉` for a `k`-step add chain; the sum of operand bounds for `mul`)
  and refuse when it approaches `FieldElement::max_num_bits()`. Note this bound counts *upward* growth from
  `bit_size` only; it says nothing about a representative sitting near `p`, which is why that case is excluded by C0
  rather than bounded here. A blunt, defensible v1 alternative: exclude `u128`, and exclude `mul` entirely — both as
  the elided target and as a propagation step.
- **C6 — accept the attribution move, and prove it in the snapshots.** `nargo`'s execution-failure tests snapshot the
  **full stderr per runtime** (`tooling/nargo_cli/tests/execute.rs:215-239` → `snapshots/execution_failure/<test>/
  execute__tests__{acir,brillig}_stderr.snap`), and those snapshots carry the source span and call stack, not just the
  message (e.g. `regression_8195`). So every failure-attribution change this optimization causes will surface as a
  reviewable ACIR-snapshot diff with the Brillig snapshot unchanged. That is the honest way to present the trade to
  upstream; it is also why the PR must regenerate and *explain* each moved snapshot rather than accepting it silently.

**Why the flip is sound under C0–C6, stated once.** Let `S` be the constraint system and `C_r` the elided range
constraint. The claim is `S \ {C_r} ⊨ C_r`: the covering constraint bounds a descendant `d < 2^n`, the chain relations
`d = r + w₁ + …` (or `r · c`, `c ≥ 1`) are all still in `S \ {C_r}` and are exact field arithmetic (C3, C5), each step
is non-decreasing (C2), and the cover is active exactly when the elided check was (C4). Hence `r ≤ d < 2^n`. The whole
argument is over `r` as a **non-negative integer** whose only escape from its type is *upward* — that is what C0 buys,
and it is not a formality: for an underflowing `sub` target the representative is `p - k`, the integer ordering is
destroyed by the reduction mod `p`, and `r ≤ d` is simply false (C0). Note this
argument is *global over the constraint system* — it does not care how many other consumers `r` has, which is why the
pass needs no use-list and no aliasing analysis. What it does care about, absolutely, is that the covering check
**survives every later pass** (§5).

## 5. HEAD prerequisite: the pass's private max-bits helper understates unchecked ACIR arithmetic

`checked_to_unchecked::required_bit_size` (`checked_to_unchecked.rs:126-171`) is a near-copy of
`DataFlowGraph::get_value_max_num_bits`; its doc comment says the logic is *"almost the same … except that"* it handles
bool multiplication and memoizes. That comment is **stale**: `get_value_max_num_bits` has since grown the conservative
ACIR-unchecked-arithmetic arm quoted in §2 (`dfg.rs:586-617`, with a termination regression test at `dfg.rs:1213`), and
`required_bit_size` has **not** — its fallback arm returns the static type width for any binary instruction
(`checked_to_unchecked.rs:160`).

Measured at HEAD, on the SSA below (`cargo test -p noirc_evaluator`, interpreter differential):

```text
acir(inline) fn main f0 {
  b0(v0: u32, v1: u32):
    v2 = unchecked_mul v0, v1     // field-exact, up to (2^32-1)^2 ≈ 2^64
    v3 = cast v2 as u64           // widening cast keeps the extended bits
    v4 = add v3, v3               // checked u64 add — CAN overflow
    return v4
}
```

- the two helpers disagree: `required_bit_size(v2) = 32` vs `get_value_max_num_bits(v2) = 64`; same for `v3`;
- `checked_to_unchecked` **flips `v4` to `unchecked_add`** — `32 < 64` satisfies its no-overflow rule;
- the interpreter shows the flip is observable: before `Err(Overflow { operator: Add { unchecked: false }, … })`,
  after `Ok(36893488130239234050)` — a u64-typed value of ≈ 2⁶⁵, i.e. out of range for its own type.

This violates the pass's own module contract (*"turn checked … into unchecked ones if it's guaranteed that the
operations cannot overflow"*, `checked_to_unchecked.rs:1-2`). Honest scoping of the claim:

- **Verified:** the helper divergence, the flip, and the observable execution difference, at HEAD, from hand-written
  SSA.
- **NOT verified:** that a *Noir source program* can reach this on HEAD today. Every `unchecked_*` HEAD produces from
  this pass is provably in range by construction; other producers exist (`expand_signed_checks.rs:95,121-123`,
  `expand_signed_math.rs`, `remove_bit_shifts.rs:189-384`, `flatten_cfg.rs:1638`, `die/array_oob_checks.rs:393-402`)
  and were **not** audited for an unsigned out-of-range result feeding a widening cast. So this may be latent on HEAD.
- **Certain either way:** the moment #7161 lands, out-of-range unsigned unchecked values become routine in ACIR, and
  this stops being latent. Worse, the failure mode is precisely the one that destroys §4's argument — the pass could
  flip the **covering** op to unchecked and leave an already-elided intermediate with no cover at all.

**Precedent, and a coordination note:** this failure class is not new — `noir-optimization-program.md` §2 records that
@jeswr's open #12927 shipped *"four soundness fixes … incl. unchecked add/mul exceeding its type width through a
widening cast"* inside its own range analysis. The same hazard is what §5 finds still present in HEAD's
`checked_to_unchecked` helper. That is corroboration, and a reason to check #12927's fate before writing a competing
fix (open question 2 in §9).

**Therefore: fixing `required_bit_size` is a hard prerequisite of the elision, and is worth landing as its own tiny,
independently-reviewable upstream PR** (delegate to `get_value_max_num_bits`, or lift its ACIR-unchecked arm into the
memoized copy and keep the bool-mul refinement). It is also exactly the kind of small, evidence-backed fix the program's
§8 sequencing wants early.

A second, weaker candidate gap was noticed and is **not** claimed: `get_value_max_num_bits`'s `Cast` arm takes
`value_bit_size.min(original_bit_size)` (`dfg.rs:579-585`), so a widening cast of an out-of-range unchecked value
reports the *target type width* — and `can_simplify_arithmetic_identity`'s doc comment asserts casts *"truncate/extend
to the target type"* (`simplify/binary.rs:333-334`), which the interpreter's cast semantics contradict
(`interpreter/mod.rs:625-628`). Whether that is reachable, and whether it lets `x + 0 → x` drop a live check, needs its
own repro before anyone reports it. Captured as follow-up, not asserted here.

## 6. Does it pay rent? Measured candidate frequency

Upstream closes small passes on maintenance cost (`noir-optimization-program.md` §4.2: PR #11580, *"doesn't have enough
optimizations to warrant the upkeep"*), so frequency is a gating question, not a nice-to-have. An env-free counter was
compiled into `checked_to_unchecked` (candidate detection only — C1(a)/C1(b), C2's monotone-step test, C4's
predicate-region test, same bit width, same block) and `nargo compile` was run over **509 packages**
(`test_programs/execution_success`) plus the 9 `test_programs/benchmarks` packages.

| measure | before the pass | after the pass's own flips |
|---|---|---|
| packages compiled | 509 | 509 |
| packages containing any checked unsigned `add`/`mul` at this point in the pipeline | 53 | 51 |
| checked unsigned `add`/`mul` ops seen | 2551 | 2516 |
| **C1(a) chain candidates** (later checked op covers) | **503** | **501** |
| **C1(b) `RangeCheck`-covered candidates** | **0** | **0** |
| packages with ≥1 candidate | 9 | 9 |

The second column is the number that matters: HEAD's existing rules absorb only **2** of the 503 candidates, so the
opportunity is almost entirely incremental — **501 structural chain candidates (an upper bound, not elidable checks),
≈20% of the 2516 checked unsigned `add`/`mul` ops that survive `checked_to_unchecked` today** — but it is concentrated
in **9 of 509 packages**. Throughout this record *candidate* means "passes the structural tests the counter ran";
*elidable* is reserved for a check shown to satisfy **all** of C0–C6, which nothing here does. Separately, 8 of the 9
`test_programs/benchmarks` packages have **zero** checked unsigned ops at all (they are Field/Poseidon-shaped, so this
optimization cannot touch them); the ninth, `bench_eddsa_poseidon`, does not compile at HEAD in this environment and
was not measured.

Top packages, and the shapes behind them:

| package | candidates (post-pass) / checked ops | shape |
|---|---|---|
| `conditional_1` | 360 / 482 | u32/u8 arithmetic across unrolled loops under predicates (sort + hash) |
| `regression_4449` | 69 / 138 | `let y = x + i; … [y, x, 32, 0, y + 1, …]` in a 70× unrolled loop |
| `two_array_chain_mutation` | 29 / 30 | `ptr += enable[i] as u32` — an **accumulator incremented in an unrolled loop**, the canonical shape |
| `a_6_array` | 14 / 28 | array-index arithmetic |
| `hint_black_box` · `fold_complex_outputs` · `simple_bitwise` · `conditional_regression_661` · `vectors` | 12 · 10 · 3 · 3 · 1 candidates | `simple_bitwise` is a five-`add` chain in one expression (`(i as u8) + (j as u8) + … + z`) |

Read this as an **upper bound**, honestly:

- it does not evaluate C3 beyond "the operand is used directly by the covering op" (which does exclude an intervening
  truncate on the chain edge itself), C5 at all, or whether the covering op survives later passes;
- it does not apply C0: the counter admits a candidate whose elided op is a checked `add` **or** `mul`, so the
  add-only v1 set is smaller than 501 by an unmeasured amount. (No `sub` target is in the count either way — `sub` was
  never counted, so C0's exclusion of it removes nothing from this figure.);
- candidates are counted per chain edge, so a `k`-step chain yields `k` candidates — which is the right shape (all but
  the last check are potentially elidable) but makes a single unrolled loop dominate the total;
- 509 packages of upstream test programs are a *compiler* corpus, not an application corpus. The sparq `zk/` face-repo
  circuits were not measured here.

Both runs (pre-pass-only, then pre+post) produced identical pre-pass totals, so the counter is deterministic across
compilations. The first run silently under-reported until `nargo compile --force` was used — `nargo` skips the pipeline
entirely when the artifact is up to date, which is worth knowing for any future instrumented corpus sweep.

## 7. What is not known, and the three ways this could still be worth nothing

1. **No gate measurement exists** (`bb` absent). Per the program's own §5.1/§4.2 lesson (PR #10159: an "obvious"
   improvement produced *more* opcodes), **no win may be claimed until `bb gates -s ultra_honk` shows it.** Each elided
   check removes one `bit_size`-bit range constraint plus the witness materialization it forces
   (`acir_context.rs:1146-1149`) and, under a predicate, one multiplication — that is the *expected* saving, not a
   measured one.
2. **The counter-risk is expression growth.** A range check is one of the sites that forces an accumulating ACIR
   expression into a fresh witness; eliding it leaves the intermediate as a symbolic expression that every consumer
   re-expands. That is precisely the mechanism behind noir#4629 (see `noir-acir-expression-growth-4629.md`: *"unsigned
   accumulation is already cut once per iteration by the overflow `RangeCheck`"*). On long unrolled accumulator chains
   — the exact shape §6 says dominates the candidate count — eliding every intermediate check may **grow** the circuit
   through the CSAT width transformer. This must be measured on `two_array_chain_mutation`-shaped fixtures before the
   design is trusted, and it may cap the win at "elide all but every *n*-th check".
3. **A cheaper alternative covers part of the same ground.** For chains whose steps are *constants* — the issue's own
   example — folding `add(add(v0, 1), 1) → add(v0, 2)` removes both the intermediate check *and* an addition, with the
   same failure set (monotone in the constant). HEAD has no such rule (`simplify/binary.rs:86-108` has only identity
   and zero rules), and it is upstream issues #8055/#8317, whose prior-art PR #8064 was closed unmerged. If the
   measured win concentrates on constant chains, the right PR may be that one, not this one.

## 8. Recommendation

**Proceed, but in four separately-reviewable steps, and stop after step 2 if the gate measurement does not reproduce.**
Each step is a future bead under `sq-uuvac`; none of them arms anything (upstream merge is the only "arm").

1. **Prerequisite fix (small, independent, land first).** Make `checked_to_unchecked::required_bit_size` agree with
   `DataFlowGraph::get_value_max_num_bits` for ACIR unchecked arithmetic; add the §5 SSA test (fails on HEAD, passes
   patched) plus the interpreter differential. Ships value on its own regardless of #7161's fate, and matches the
   program's "small maintainer-blessed opener" sequencing.
2. **Measurement spike before the pass.** Build fixtures for the §6 shapes (a five-`add` expression chain; a 32-iteration unrolled
   accumulator; a predicated chain), hand-elide them at SSA level, and take `bb gates -s ultra_honk` before/after plus
   ACIR opcode counts. This isolates the win — and the §7.2 expression-growth counter-risk — *without* writing the
   pass. **Gate:** no reproducible gate win ⇒ record a null result and park, as `sq-seust` did. Note §6 already
   settles that the existing benchmark suite will show **zero** movement, so a focused fixture under
   `test_programs/benchmarks/` is mandatory for the PR (the same shape jeswr's `live_bit_width_*` fixtures take), and
   the "9 of 509 packages" concentration is the rent question a reviewer will ask first.
3. **The pass itself**, ACIR-only, C1(a)-only, and in v1: C0 (the elided op is a checked unsigned **`add`** — `sub`
   excluded for the C0 soundness reason, not deferred), `u128` excluded, `mul` excluded **both** as the elided target
   and as a propagation step in the chain, and C4 as "no `EnableSideEffectsIf` in between". Acceptance per
   `noir-optimization-program.md` §10.2, plus: the §3 interpreter differentials as unit
   tests; every moved `execution_failure` ACIR snapshot explained in the PR body with its Brillig counterpart shown
   unchanged; a `--force-brillig` run proving Brillig behaviour is untouched.
4. **Only then** consider C1(b) (`RangeCheck`-covered), `mul` chains with a magnitude analysis, and the `u128` case —
   each gated on its own measured win. A `sub` target is **not** on this list: it would need a different argument
   altogether (one that reasons about representatives near `p`, i.e. proves `a ≥ b` independently), not a widening of
   the C2/C5 magnitude bounds. Note C1(b)'s soundness depends on the covering `RangeCheck` surviving
   `simplify.rs:281-284`, which removes a `RangeCheck` when `get_value_max_num_bits(value) ≤ max_bit_size`; the
   conservative arm from §2 is exactly what keeps it alive, which is a second reason step 1 comes first.

**Dependency note:** `noir-optimization-program.md` §10.4 records `sq-jthy1` as blocking the flagship `sq-jj3ne`
(#9429-style value-range analysis) because they share `checked_to_unchecked.rs`. Step 1 above is the natural merge
point of the two: a shared, conservative max-bits query is the thing both need.

## 9. Open questions for the maintainer

1. **Is the failure-attribution move acceptable to upstream at all?** Every elided check moves an
   `"attempt to add with overflow"` diagnostic from the user's actual overflowing line to a later one, and makes ACIR
   and Brillig disagree about where a program failed. That is a UX regression traded for gates. It is upstream's call,
   and it should be asked **on the issue before code is written** — the issue's own "to be determined by whoever works
   on this" is an invitation to ask.
2. **Does @jeswr's open #12780/#12927 range analysis already subsume this?** Those PRs add the global range facts that
   would make this elision a consumer rather than a standalone pass. Per program §8 step 3, duplicate PRs are the
   thing to avoid; this record's step 1 is deliberately the piece that is useful either way.
3. **Should step 1 (the `required_bit_size` fix) be reported as a bug or shipped as a hardening?** It is a contract
   violation with a measured repro but no demonstrated Noir-source reachability (§5). Reporting it as "unsound today"
   without that reachability would overclaim.

## 10. Reproduction

Environment (the recipe from `noir-not-canonicalization-ifelse-merging.md` §8, confirmed again here): the box's
`rustup` root is read-only, so install the pin into a writable one —
`RUSTUP_HOME=<writable> rustup toolchain install 1.89.0-x86_64-unknown-linux-gnu --profile minimal`, then build with
`RUSTUP_TOOLCHAIN=1.89.0-x86_64-unknown-linux-gnu` and a `CARGO_TARGET_DIR` outside `/tmp` (the noir test profile is
optimized; the lib-test build and a `nargo` dev build are each a few minutes on 4 cores). `bb` is still absent, so gate
counts remain out of reach on this box.

- **§3 semantics table:** a temporary `#[test]` in `checked_to_unchecked.rs`'s test module calling `Ssa::from_str` on
  each program and `ssa.interpret(vec![Value::u32(u32::MAX)])`, printed with `-- --nocapture`.
- **§5 counterexample:** the same, plus `required_bit_size` and `dfg.get_value_max_num_bits` printed for `v2`/`v3`.
- **§6 frequency:** a candidate counter compiled into `Function::checked_to_unchecked` (atomics + one `eprintln!` per
  compilation at `Ssa::checked_to_unchecked`), a dev-profile `nargo`, and
  `nargo compile --force --silence-warnings` over every `test_programs/execution_success` package, 4-way parallel
  (`--force` is required: `nargo` skips the whole pipeline when the artifact is up to date, so an instrumented sweep
  silently reports nothing on a second run). The post-pass column calls the same counter again after the pass's own
  rewrites.

All instrumentation lives in the throwaway clone. Nothing from it is carried in this repo, and no upstream PR is
proposed by this record.
