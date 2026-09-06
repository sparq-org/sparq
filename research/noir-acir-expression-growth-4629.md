# Quadratic ACIR expression growth in loops — noir#4629 design note (sq-felqr)

**Bead:** `sq-felqr` (epic `sq-uuvac`, `noir-optimization-program.md` §7 row 10, §10.3 "design note on
noir#4629 only; NO implementation before upstream buy-in") · **Status:** **design note delivered;
bead NOT complete** — the empirical half (§7) was not run and the live issue thread was not read
(§0), and per the bead's own exit condition no implementation may start before upstream buy-in ·
**Upstream comment DRAFTED, NOT posted** — awaiting @jeswr review per `AGENTS.md`
§ *Upstream contributions* · **Author:** SPARQ agent 🤖 [OPUS-5] · **Date:** 2026-07-27

Analysed against `noir-lang/noir` @ `e22cd89b` (2026-07-24, workspace version `1.0.0-beta.25`).
Every line citation below is at that commit, in a throwaway clone outside this repo. Companion
records: `noir-optimization-program.md` (§3.3 ACIR-gen map, §4.1 issue table, §10 fleet spec) and
`noir-investigation-on-ramps-upstream.md` (the sibling findings-only bead, same environment limits).

## 0. What this record is, and what it is not

`sq-felqr` is explicitly a **design-first** bead: the program record rates row 10 "high effort/risk"
and its fleet spec says *design note only, no implementation before upstream buy-in*. The
deliverable is therefore an analysis of where the growth comes from, what the candidate cut points
are, and what would have to be true for a patch to be worth proposing — not a patch.

Environment limits, stated up front because they bound every claim below:

| capability | available | consequence |
|---|---|---|
| noir source at HEAD | yes (clone @ `e22cd89b`, full tree) | full source analysis possible |
| build of `noirc`/`nargo` | **no** — workspace pins rustc `1.89.0` (`rust-toolchain.toml`); only `1.88.0` is installed and `rustup` cannot install (read-only `/usr/local/rustup`). `cargo check -p noirc_evaluator` stops at *"noirc_evaluator@1.0.0-beta.25 requires rustc 1.89.0"* | no ACIR opcode counts |
| `bb` | **no** | no gate counts |
| noir issue/PR bodies (#4629, #6539, #13046) | **not fetched** — no outbound page-fetch permission in this run | the *ask*, the reproducer and the thread history are taken from `noir-optimization-program.md` §4.1 + the bead text, **not** from the live issues |
| noir git history | **no** — depth-1 clone | cannot check whether upstream already attempted a fix |

So this is a **source-derived** design note. **Nothing here is a measured claim**, and per the
program's own doctrine (§2, and the PR #10159 lesson — *measure gates, not intuition*) nothing here
may be asserted upstream as a measurement. Two consequences are load-bearing and are repeated in
the drafted comment (§8):

1. **Blocking pre-check.** The reproducer in #4629 was not read. §4 below derives, from the
   lowering code, *which program shapes are quadratic by construction on HEAD* — but which of them
   the issue actually reports is unknown, and #4629 predates the current ACVM
   `CommonSubexpressionOptimizer` (§5), so **the first step is to re-run the issue's own reproducer
   at HEAD and confirm the bug still reproduces at all**. Designing a heuristic for a bug that a
   later refactor already closed would be the expensive mistake here.
2. **No implementation is proposed.** §6 ranks options; it does not pick a patch. The bead's exit
   condition (upstream buy-in on the issue thread) is not met and is not applied by this record.

## 1. The ask

From the program record §4.1: *"ACIR size grows quadratically with loop iterations (SSA is linear ⇒
acir-gen expression growth)"*, with `as_witness` added as the manual workaround and #6539 recorded
as a duplicate. The bead sketch adds the framing to test: *find where accumulated expressions
should be cut into intermediate witnesses automatically — a width-aware auto-`as_witness` heuristic
at acir-gen, or in the CSAT transformer.*

The framing is well-founded in upstream's own documentation. `std::as_witness` is declared in
`noir_stdlib/src/lib.nr:115-119` as *"Force a field value to be a witness instead of an expression …
often only useful for debugging compiler optimizations"*, and the user-facing guide
(`docs/docs/guides/thinking_in_circuits.md:210`) calls it

> a temporary lever (that will become unnecessary/useless over time) … The compiler will mostly be
> correct and optimal, but this may help some near term edge cases that are yet to optimize.
> Note: When used incorrectly it will create **less** efficient circuits (higher gate count).

That is upstream stating both halves of this bead: the automation is the intended direction, *and*
the operation is not free — a misplaced cut costs gates. Any heuristic is therefore judged on its
worst case on well-conditioned circuits, not on its best case on the pathological one.

## 2. Mechanism: how an ACIR expression grows

ACIR-gen is a symbolic evaluator, not an emitter. An `AcirVar` maps to
`AcirVarData::{Const, Witness, Expr}` (`acir/acir_context/mod.rs:1560-1568`), where `Expr` is a full
ACIR `Expression` (multiplication terms + linear combination + constant). Reading a var
(`var_to_expression`, `:261-268`) returns `into_owned()` — **a clone**.

Addition never materialises anything (`add_var`, `:746-757`):

```rust
let lhs_expr = self.var_to_expression(lhs)?;
let rhs_expr = self.var_to_expression(rhs)?;
let sum_expr = &lhs_expr + &rhs_expr;
let sum_var = self.add_data(AcirVarData::from(sum_expr));
```

There is no width bound, no term-count check, and no reference to any backend width anywhere in
`acir_context`. So an unrolled `acc = acc + f(x[i])` over `n` iterations yields **one** `AcirVar`
whose expression carries `O(n)` terms — and, because each step clones both operands, the *building*
of it costs `O(n²)` term copies and allocations regardless of what is finally emitted.

Multiplication is where the symbolic evaluation is forced to give up (`mul_var`, `:650-733`). It has
fast paths for constant×anything, witness×witness, linear-expr×witness (distributes into `k`
multiplication terms) and linear×degree-one-univariate; **everything else** falls to

```rust
let lhs = self.get_or_create_witness_var(lhs)?;
let rhs = self.get_or_create_witness_var(rhs)?;
self.mul_var(lhs, rhs)?
```

(`:722-730`) — i.e. the degree bound of ACIR, not any size heuristic, is what cuts today.

## 3. Where a cut happens today, and why `as_witness` works

The cut primitive is `get_or_create_witness_var` (`:222-244`): it materialises the expression as a
witness and then calls `mark_variables_equivalent` (`:144-208`), whose `(Witness, Expr)` arm —
*"Replace any expressions with witnesses"* (`:174-178`) — **rewrites `self.vars[var]` in place** from
the wide `Expr` to the narrow `Witness`. That in-place rewrite is the whole trick: every *later*
read of that same `AcirVar` sees a one-term expression.

`std::as_witness` is exactly that one call and nothing else (`acir/call/intrinsics/mod.rs:133-140`):

```rust
Intrinsic::AsWitness => {
    let arg = arguments[0];
    let input = self.convert_value(arg, dfg).into_var()?;
    // `as_witness` exists only for its side effect of creating a witness for the input
    self.acir_context.get_or_create_witness_var(input)?;
    Ok(Vec::new())
}
```

**Design consequence: the mechanism for an automatic cut already exists and is one call.** The open
question is purely *where and when to call it* — which is a heuristic/policy question, not a
plumbing one. That is a genuinely favourable starting position for a patch, and it is why the risk
in this bead is concentrated entirely in the heuristic.

Every cut site in ACIR-gen at HEAD (exhaustive, from `get_or_create_witness_var` / `var_to_witness`
call sites):

| site | file:line | narrows the source var? |
|---|---|---|
| `mul_var` non-representable fallback | `acir_context/mod.rs:722,728` | yes |
| `range_constrain_var` | `:1147` | yes (see below) |
| memory read/write index + value | `:1378-1405` | yes |
| array initialisation elements | `:1463` | yes |
| black-box function inputs | `black_box.rs:257,287` | yes |
| brillig **outputs** | `brillig_call.rs:102` | n/a (fresh witnesses) |
| `Intrinsic::AsWitness` | `call/intrinsics/mod.rs:138` | yes — this is the manual lever |

None of these is width-driven; each fires because *that consumer needs a `Witness` rather than an
`Expression`*.

`range_constrain_var` deserves a note because it is the reason a large class of loops is **not**
quadratic today. It computes `predicate_range = mul_var(variable, predicate)` and narrows that
(`:1145-1148`); the sole caller (`acir/mod.rs:546-556`, `Instruction::RangeCheck`) always passes the
constant `one` because the predicate was already folded in during flattening, and `mul_var`'s
`(_, Const(1)) => lhs` arm (`:663`) returns *the same var* — so the narrowing lands on the
accumulator itself. Every unsigned add/sub/mul in a loop carries an overflow `RangeCheck`, so
**unsigned accumulation is cut once per iteration, for free, already**. (Note also that
`range_constrain_var` returns `predicate_range` rather than `witness_var` at `:1157-1162`; that is
harmless while the predicate is constant `one`, and would be a missed narrowing if a caller ever
passed a non-constant predicate.)

## 4. What is quadratic by construction on HEAD — and what is not

Derived from the lowering, not measured. The distinction that matters is between shapes where a
growing expression is **consumed once at the end** (linear) and shapes where a *copy* of the growing
expression is **emitted every iteration** (quadratic).

**Q1 — repeated equality assertions against a growing accumulator.** `assert_eq_var`
(`:508-540`) builds `diff_expr = lhs_expr - rhs_expr`, hands it to `assert_is_zero`, and then calls
`mark_variables_equivalent(lhs, rhs)` — which for two `Expr` operands creates **no** witness and
only picks the smaller of the two when *both* are linear (`:185-199`). So `n` assertions over
prefixes of an accumulator emit `n` opcodes of size `O(i)`: `Θ(n²)` total opcode terms, with no
narrowing of the accumulator at any point.

**Q2 — brillig-call and assertion-payload inputs.** `BrilligInputs::Single(self.var_to_expression(var)?)`
(`brillig_call.rs:66-69`) embeds a **full copy** of the expression in the call opcode, and
`values_to_expressions_or_memory` (`acir_context/mod.rs:601-621`) takes `&self`, so it *structurally
cannot* cut even if it wanted to. Any loop that feeds a growing accumulator to an unconstrained
helper — including every implicit brillig hint — pays a full copy per call.

**Q3 — compile time and peak memory, independent of the emitted circuit.** Because `var_to_expression`
clones (`:261`) and `add_var` allocates a fresh `O(i)`-term expression per step (`:746`), building an
`n`-term accumulator is `Θ(n²)` in term copies *even when the final ACIR is linear*. This is the same
root cause as the memory-blowup issue #13046 (program record §4.1: *"Memory blowup during ACIR
synthesis (Poseidon sponge)"*) and it is invisible to any post-hoc ACVM pass, because the peak is
reached before ACVM ever sees the circuit.

**Not quadratic (already cut):** unsigned arithmetic in loops (§3, the `RangeCheck` path); anything
whose accumulator reaches a multiplication that ACIR cannot represent symbolically (`mul_var`
fallback); memory reads/writes; black-box inputs.

This split is the most actionable output of this record: **the automatic cut is only needed at the
few consumers that do not already narrow (Q1/Q2), which is a far smaller and lower-risk change than
a global width-aware heuristic in `add_var`.**

## 5. What the ACVM passes already do — and why the circuit-size win may be near zero

Downstream of ACIR-gen, `CommonSubexpressionOptimizer`
(`acvm-repo/acvm/src/compiler/optimizers/common_subexpression/mod.rs`) runs up to
`DEFAULT_MAX_TRANSFORMER_PASSES = 3` rounds (`:87`, `:121-140`) of:

1. **`CSatTransformer`** (`csat.rs`) slices each `AssertZero` opcode towards
   `DEFAULT_EXPRESSION_WIDTH = 4` (`mod.rs:88`, `:168`), hoisting chunks of at most `width - 1`
   *solvable* linear terms into intermediate witnesses (`partial_opcode_scan_optimization`,
   `csat.rs:354-446`), and recursing until the fan-in fits.
2. **`MergeExpressionsOptimizer`** (`merge_expressions.rs:30-57`) then **undoes** any intermediate
   used in exactly two `AssertZero` opcodes, by Gaussian elimination — because the backend handles
   wide linear combinations better than extra witnesses.

Two properties of this pipeline bear directly on the design:

**(a) The CSAT intermediate cache is circuit-global.** `intermediate_variables` is declared once,
before the opcode loop (`mod.rs:180-182`), and every slice is normalised by its leading coefficient
before lookup (`csat.rs:267-289`, `get_or_create_intermediate_var` `:293`). ACIR `Expression`s are
sorted by witness index, so `n` opcodes carrying *prefixes* of the same accumulator present the same
leading `width - 1` terms and should hit the same cache entry, recursively — which would collapse a
quadratic pile of prefix opcodes towards a shared `O(n)` tree of intermediates. **Predicted from the
code, not measured** — and it is precisely why §0's blocking pre-check exists: this pass
postdates #4629, and it may already have absorbed most of the circuit-size half of the issue.

**(b) An over-eager cut is partially self-healing.** A witness introduced by a speculative
auto-`as_witness` that appears in exactly two `AssertZero` opcodes is merged back by
`MergeExpressionsOptimizer`, so its cost is refunded. **The opcode accounting is easy to get off by
one:** materialising the expression emits the `AssertZero` that *defines* the witness, and the
elimination consumes that defining equation to substitute — so "two opcodes" is the definition plus
**exactly one** consumer. A cut with **two or more** `AssertZero` consumers is already at three or
more opcodes and is **not** merged back. The refund also does **not** apply when the extra witness
is consumed by a non-`AssertZero` opcode (memory op, black-box call, brillig input) — there the cut
is permanent and costs a gate. (Source-derived like everything else here; re-confirm the counting
against `merge_expressions.rs:30-57` before any of these numbers is asserted upstream.) That is exactly the docs'
*"when used incorrectly it will create less efficient circuits"*, and it gives the heuristic a
concrete safety argument plus a concrete list of places where the argument fails.

**Reframing.** Taken together, (a), (b) and Q3 suggest the honest statement of the problem is not
one issue but two, with different owners:

- a **circuit-size** problem (Q1/Q2), which may already be largely absorbed by the ACVM CSE pass and
  must be re-measured before anyone designs for it;
- a **compile-time / peak-memory** problem (Q3), which ACVM cannot touch by construction, which is
  where `as_witness` demonstrably helps users today, and which is the same root cause as #13046.

If the measurement supports it, the highest-value upstream contribution from this bead may be to say
so on the thread — the same kind of re-scoping the sibling record produced for #6313.

## 6. Candidate designs, ranked (no implementation proposed)

| # | Design | Where | Pro | Con / risk |
|---|---|---|---|---|
| A | **Narrow at the non-narrowing consumers**: call `get_or_create_witness_var` on the operands of `assert_eq_var` and on brillig `Single` inputs when the expression exceeds a term threshold | `acir_context/mod.rs:508`, `brillig_call.rs:66` | smallest diff; targets exactly Q1/Q2; reuses the existing primitive; leaves every already-cut path untouched | `values_to_expressions_or_memory` is `&self` and would need to become `&mut self`; changes assertion opcode shape ⇒ SSA/ACIR snapshot churn; needs a threshold (see D) |
| B | **Width-aware auto-cut in `add_var`**: cut when `linear_combinations.len()` exceeds `k · width` | `acir_context/mod.rs:746` | single choke point; catches Q3 (the memory/compile-time half) as well, since the accumulator never grows past the threshold | ACIR-gen has **no notion of backend width today** (`ExpressionWidth` no longer exists in the workspace; the width is the ACVM-private `DEFAULT_EXPRESSION_WIDTH`) — the threshold would have to be plumbed or duplicated; cuts unconditionally, including on values ACVM would rather keep wide; highest regression risk on well-conditioned circuits |
| C | **CSAT-side**: make slice alignment canonical so shared prefixes always hit the cache | `csat.rs:354` | no ACIR-gen change; no new gates by construction | per §5(a) this may **already** hold — must be measured before it is designed; and it cannot address Q3 at all |
| D | **Use SSA use-counts to place the cut**: insert `AsWitness` in SSA where a value has enough uses to survive the §5(b) merge-back *and* a large ACIR-gen cost estimate | new SSA pass | reuses the documented lever; uses information ACIR-gen does not have (the DFG knows use counts) | the SSA use count is **not** the quantity that governs the refund: per §5(b) a cut survives from **two eligible `AssertZero` consumers** (three opcodes counting the definition), and SSA uses do not map one-to-one onto those — a use may lower to a memory/black-box/brillig operand (never merged back at all), several uses may fold into one opcode, and CSAT slicing adds opcodes after ACIR-gen — so the threshold would be a proxy the pass cannot validate at its own layer; also requires an ACIR-cost model in SSA, i.e. a layering inversion, and SSA is deliberately width-agnostic; a new pass must "pay rent" (PR #11580 was closed on maintenance-cost grounds — program record §4.2) |

**Preliminary ordering, contingent on §7:** if the measurement shows the circuit-size blowup is real
at HEAD → **A** first (smallest, targeted, and its cost is bounded by §5(b)). If the measurement
shows circuit size is already fine but compile time / peak memory is not → the problem is Q3, only
**B** touches it, and the framing to take upstream is the #13046 one, not the #4629 one. **C** is a
measurement question before it is a design. **D** ranks last: its cut threshold is an SSA use-count
proxy that §5(b) does not underwrite — the merge-back is decided by `AssertZero` opcode occupancy
after lowering, not by DFG use count — so the proxy would have to be validated empirically before
the design means anything, and it is also the least likely to be accepted as a first contribution.

## 7. The pending empirical half — exact protocol

Needs a box with rustc `1.89.0`, `nargo` and `bb` (this run had none, §0). Baseline: noir
@ `e22cd89b`, `bb gates -s ultra_honk`, plus the program record's §10.2 acceptance protocol
(differential `nargo execute --force` vs `--force-brillig`, corpus regression, compile-time sanity
on `sha512_100_bytes`).

1. **Re-read #4629, #6539 and #13046** (bodies *and* threads) and recover the issue's own
   reproducer. Blocking: everything below is scoped by which shape the issue reports.
2. **Confirm the bug still exists at HEAD.** Run the issue's reproducer at
   `n ∈ {8, 16, 32, 64, 128}` and fit the growth of (i) `nargo info` ACIR opcodes, (ii) `bb gates`,
   (iii) `nargo compile` wall-clock, (iv) peak RSS. **A super-linear fit in (i)/(ii) is what the
   issue claims; if only (iii)/(iv) are super-linear, the issue is Q3 and should be re-scoped on the
   thread** (and merged with #13046 rather than fixed with a width heuristic).
3. **Localise the shape.** Run the same sweep over one fixture per §4 shape — Q1 (`assert` on a
   growing `Field` accumulator each iteration), Q2 (accumulator passed to an unconstrained helper
   each iteration), Q3 (pure `Field` accumulation, consumed once), plus the unsigned control which
   §3 predicts is already linear. The control going linear is the falsifiable part of §3.
4. **Quantify the ACVM contribution** (tests §5(a)): dump ACIR before and after
   `compiler::optimize`/`transform` on the quadratic fixture and count opcodes at each stage. If the
   post-ACVM count is linear while the pre-ACVM count is quadratic, option C is moot and options A/B
   are compile-time fixes only — which changes what to say upstream.
5. **Bound the manual lever:** re-run the reproducer with a hand-placed `std::as_witness` in the loop
   body and record the same four metrics. That is the ceiling any automatic heuristic can reach, and
   the honest number to put on the thread.
6. **Only then**, if a win survives, propose a design on the issue and wait for maintainer
   engagement before writing a patch (bead exit condition; program record §6 and §10.3).

Decision rule for any later PR, inherited unchanged from the program record §5.1/§8: report a win
only if `bb gates` strictly decreases on at least one benchmark and increases on none, with the
corpus recompiled and no unexplained ACIR increase.

## 8. Drafted upstream comment (NOT posted)

> **DRAFT — not posted.** Requires @jeswr review per `AGENTS.md` § *Upstream contributions*.
> **Blocking pre-checks before this may be posted:** (a) read the #4629 body and thread — this
> analysis did not; (b) re-run the issue's reproducer at HEAD and confirm it still reproduces, since
> the ACVM `CommonSubexpressionOptimizer` postdates this issue; (c) replace the qualitative claims
> below with measurements or delete them.
>
> > 🤖 Posted by an autonomous agent (a SPARQ agent) operating on @jeswr's behalf.
>
> Some notes from reading the ACIR-gen path at `e22cd89b`, in case they help scope this.
>
> The growth mechanism is that `add_var` (`acir_context/mod.rs:746`) concatenates expressions with no
> width bound, and the only cut points are consumers that need a `Witness` rather than an
> `Expression` — the `mul_var` fallback (`:722`), range checks (`:1147`), memory/black-box inputs, and
> `as_witness` itself (`call/intrinsics/mod.rs:138`). All of them go through
> `get_or_create_witness_var`, which rewrites the var in place (`mark_variables_equivalent`, `:174`),
> so the automatic version of `as_witness` would be the same one call — the open question is only
> where to place it.
>
> Two things that seem worth checking before designing that heuristic:
>
> 1. Unsigned accumulation already gets a cut for free every iteration, because the overflow
>    `RangeCheck` narrows the accumulator via `range_constrain_var`. The shapes that look quadratic
>    by construction are narrower than "loops": repeated `assert_eq` against a growing accumulator
>    (`assert_eq_var` emits the whole expression and never narrows either side, `:508`), and values
>    passed to brillig calls (`BrilligInputs::Single` copies the full expression,
>    `brillig_call.rs:66`).
> 2. `CommonSubexpressionOptimizer` postdates this issue, and its CSAT intermediate cache is
>    circuit-global, so prefix slices of the same accumulator across many opcodes should already
>    share intermediates — and `MergeExpressionsOptimizer` refunds any intermediate that appears in
>    exactly two `AssertZero` opcodes (the one defining it plus a single consumer), which bounds the
>    cost of an over-eager cut on singly-consumed values but also means a lot of the circuit-size
>    half may already be handled.
>
> If that is right, the part ACVM cannot help with is compile time and peak memory — every
> `var_to_expression` clones, so building an n-term accumulator is quadratic in term copies even when
> the emitted ACIR is linear — which looks like the same root cause as #13046.
>
> This is an argument from the source, **not a measurement**. Happy to run the sweep (opcodes, gates,
> compile time, peak RSS at several loop counts, plus the same with a hand-placed `as_witness` as the
> ceiling) and post the numbers, if that would help decide whether this is one issue or two.

## 9. What is *not* established by this record

- No opcode count, gate count, compile time or memory number anywhere — nothing was built or
  executed (§0).
- Whether #4629 still reproduces at HEAD. Unknown, and §5(a) gives a concrete reason it might not.
- Which of the §4 shapes the issue actually reports. The issue body was not read.
- Whether #6539 is genuinely a duplicate, and what maintainers have already said on either thread.
- The §5(a) cache-sharing argument depends on prefix opcodes presenting identically-ordered leading
  terms; that follows from `Expression` sorting and CSAT normalisation as read, but was not observed.
- The cost arguments are read off the lowering code. They are not proofs of semantics preservation,
  and per the program record §4.2 (PR #10159) second-order witness-sharing effects have reversed
  source-level predictions in exactly this part of the compiler before.
- Nothing has been posted upstream, and nothing may be until @jeswr reviews the §8 draft.
