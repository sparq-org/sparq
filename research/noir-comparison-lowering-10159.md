# Comparison lowering via a specialized power-of-two split — noir experiment record (sq-jfkwk)

**Bead:** `sq-jfkwk` (epic `sq-uuvac`, `noir-optimization-program.md` §7 row 9, §10.3 *"findings first;
`acir/acir_context/mod.rs` only on a win … fail-closed on the #10159 witness-sharing landmine; null
result acceptable"*) · **Status:** **experiment run at source level; the sketch is a NULL RESULT and no
upstream PR is proposed for it.** One *adjacent* candidate was found and is specified but **not
implemented and not measured** · **Upstream comment DRAFTED, NOT posted** — awaiting @jeswr review per
`AGENTS.md` § *Upstream contributions* · **Author:** SPARQ agent 🤖 [OPUS-5] · **Date:** 2026-07-27

Analysed against `noir-lang/noir` @ `e22cd89b` (2026-07-24, workspace version `1.0.0-beta.25`), in a
throwaway clone outside this repo. Every line citation is at that commit. Companion records:
`noir-optimization-program.md` (§3.3 ACIR-gen map, §4.2 landmine list, §10 fleet spec),
`noir-acir-expression-growth-4629.md` and `noir-investigation-on-ramps-upstream.md` (the two sibling
findings-only beads, same environment limits).

## 0. What this record is, and what it is not

The bead carries an explicit **measure-first mandate**: asterite's own PR #10159 changed exactly this
function and produced *larger* circuits, so nothing may go upstream without a demonstrated `bb gates`
win across the sparq corpus *and* the upstream benchmark projects. The bead also states that a null
result is an acceptable outcome. **This record delivers a null result for the sketch as written**, and
it does so from a stronger evidence base than "intuition": upstream's own committed ACIR snapshot
tests print the exact opcode sequence a comparison lowers to, so the sketch can be evaluated
opcode-for-opcode without a compiler build.

Environment limits, stated up front because they bound every claim below:

| capability | available | consequence |
|---|---|---|
| noir source at HEAD | yes (clone @ `e22cd89b`, full tree) | full source analysis possible |
| upstream ACIR snapshot tests (committed expected output) | yes | **ground-truth opcode sequences** for `lt u8` and 128-bit truncate, without building |
| build of `noirc`/`nargo` | **no** — workspace pins rustc `1.89.0` (`rust-toolchain.toml`); only `1.88.0` is installed and `rustup` cannot install (read-only `/usr/local/rustup`) | no fresh ACIR opcode counts |
| `bb` | **no** | **no gate counts** — so no claim here is a gate measurement |
| PR #10159 body/thread, and the issues it references | **not fetched** — outbound page fetch is not permitted in this run | the #10159 outcome is taken from `noir-optimization-program.md` §4.2, **not** from the live thread; §5 below *derives* a mechanism consistent with that summary and labels it a hypothesis |
| noir git history | **no** — depth-1 clone | cannot check whether the §6 gap was already attempted upstream |

So: §2–§4 are **source- and snapshot-derived facts** about HEAD. §5 is a **hypothesis**. §6 is a
**candidate**, not a result. Nothing here is a measurement, and per the program's own doctrine
(§5.1, §6.7) nothing here may be asserted upstream as one.

## 1. The ask

Bead sketch, verbatim in substance: `more_than_eq_var` (`acir/acir_context/mod.rs:1183-1246`) extracts
the sign bit of `diff + 2^max_bits` with a **full** `euclidean_division_var` — Brillig quotient hint,
quotient range check, remainder range check, and the division relation — even though the quotient is
provably `0`/`1`. Proposed specialization: a witness bit `b` (boolean-constrained) plus a remainder
`r` (range-checked to `max_bits`) constrained by `diff + 2^max_bits == b·2^max_bits + r`, *"saves the
quotient range check"*.

The premise to test is therefore narrow and checkable: **is the quotient range check that the sketch
removes actually a cost on HEAD, and is anything else in that call redundant?**

## 2. What HEAD actually emits for a comparison

`compiler/noirc_evaluator/src/acir/tests/instructions/binary.rs:490-535` (`lt_u8`) is an upstream test
whose committed `assert_circuit_snapshot!` expectation *is* the emitted circuit for `v2 = lt v0, v1`
on `u8`:

```text
private parameters: [w0, w1]
return values: [w2]
BLACKBOX::RANGE input: w0, bits: 8                    ← parameter typing, not the comparison
BLACKBOX::RANGE input: w1, bits: 8                    ← parameter typing, not the comparison
BRILLIG CALL func: 0, predicate: 1, inputs: [w0 - w1 + 256, 256], outputs: [w3, w4]
BLACKBOX::RANGE input: w3, bits: 1                    ← the "quotient range check"
BLACKBOX::RANGE input: w4, bits: 8                    ← the remainder range check
ASSERT w4 = w0 - w1 - 256*w3 + 256                    ← the division relation
ASSERT w2 = -w3 + 1                                   ← the `lt` negation (only because w2 is returned)
```

Four opcodes for the comparison itself: one unconstrained Brillig hint (no proving cost), a **1-bit**
range check, a `max_bits`-bit range check, one `AssertZero`. That is the whole cost.

## 3. Why it is already that small — the three reductions that have already happened

The sketch's cost model ("full euclidean division") describes `euclidean_division_var`'s *code*, not
its *output* for this call. Three existing mechanisms collapse it:

**(a) The quotient range check is already 1 bit.** `euclidean_division_var:1002-1030` tightens the
bounds when `rhs` is constant: `max_q_bits = bit_size - rhs_bits + 1`. `more_than_eq_var` passes
`bit_size = max_bits + 1` and `rhs = 2^max_bits`, whose `num_bits()` is `max_bits + 1`, so
`max_q_bits = 1` — the compiler already knows the quotient is a bit. `BLACKBOX::RANGE … bits: 1` *is*
the boolean constraint the sketch would emit for `b`; the sketch does not remove it, it renames it.

**(b) The duplicate range checks are already deduplicated.** `stdlib_brillig_call` passes
`skip_output_range_checks = false` (`brillig_call.rs:17-35`), so the hint's outputs are range-checked
to their declared types (`unsigned(max_q_bits)`, `unsigned(max_r_bits)`) at
`brillig_call.rs:122-150`; `euclidean_division_var:1112-1126` then range-checks the same two witnesses
again. Both pairs are identical checks on identical witnesses, and the ACVM `redundant_range`
optimizer (`acvm-repo/acvm/src/compiler/optimizers/redundant_range.rs`) keeps only the tightest per
witness. The snapshot shows one `RANGE` per output, confirming the collapse.

**(c) The `r < rhs` bound constraint collapses into the remainder range check.** With
`rhs = 2^max_bits` and `offset = 1`, `bound_constraint_with_offset:1060-1096` takes its fast path:
`can_move_offset_to_rhs` holds, `rhs_offset = 2^max_bits - 1`, `bit_size = max_bits`, and the padding
constant is `r = (2^max_bits - 1) - (2^max_bits - 1) = 0`. So `aor = remainder + 0` — *the same
witness* — and the emitted check is `RANGE(remainder, max_bits)`, byte-identical to the one already
emitted. `redundant_range` removes it. This is precisely what the in-code comment at
`:1116-1124` anticipates, and it is why the program record §7 already lists "redundant remainder range
check in `euclidean_division_var`" as explicitly rejected. That rejection is confirmed here — with the
correction that the mechanism is *fast-path degeneration plus witness-level dedup*, not "the ACVM
optimizer removes the semantically weaker one".

The remaining machinery in `euclidean_division_var` is free for this call: the divide-by-zero guard
takes the `predicate == 1` arm and calls `inv_var` on a **constant**, which constant-folds
(`:309-330`) into an `assert_eq` of two equal constants that `assert_eq_var:508-533` discards; and the
`q·b + r` overflow guard needs `max_q_bits + max_rhs_bits ≥ F::max_num_bits() - 1`, i.e.
`1 + (max_bits+1) ≥ 253`, which no legal `max_bits ≤ 128` reaches.

## 4. Verdict on the sketch: opcode-identical, therefore a null result

Emitted by the specialized path the bead describes, for `max_bits < 128`:

| the sketch would emit | HEAD already emits | delta |
|---|---|---|
| hint producing `b`, `r` (the same `directive_integer_quotient` works) | `BRILLIG CALL` | 0 |
| boolean constraint on `b` — `RANGE(b, 1)`, or `AssertZero(b² − b)` | `BLACKBOX::RANGE … bits: 1` | 0 (an `AssertZero` form would be **worse**: a multiplication term where a 1-bit range gadget suffices) |
| `RANGE(r, max_bits)` | `BLACKBOX::RANGE … bits: max_bits` | 0 |
| `diff + 2^max_bits == b·2^max_bits + r` | `ASSERT w4 = w0 - w1 - 256*w3 + 256` | 0 |

**The specialization is opcode-for-opcode identical to what HEAD emits.** The saving in the bead
sketch does not exist: it was already realized, before this bead was written, by the constant-`rhs`
bit-bound tightening (§3a). A PR implementing the sketch would be a pure refactor of `acir_context`
that cannot win gates and can only lose them — which is exactly the failure shape §4.2 records
for #10159. Per the bead's fail-closed instruction, **no PR is proposed and `acir_context/mod.rs`
is not touched**.

Two caveats on the strength of this verdict, stated because they are the honest boundary:

1. It is an **opcode-level** identity argument, and the program's own metric hierarchy (§5.1) puts
   `bb gates` above opcodes. An opcode-identical circuit is gate-identical, so the direction of the
   conclusion is safe; but it is derived, not measured.
2. It holds for the lowering as HEAD writes it. A *different* specialization — one that changed which
   witness the comparison result is carried in — is not covered by this table, and is the subject
   of §5.

## 5. The #10159 landmine — a derived mechanism (hypothesis, not verified against the thread)

`noir-optimization-program.md` §4.2 records the outcome: PR #10159 *"produced more opcodes/larger
circuits because `a<b` and `!(a<b)` stopped sharing a witness"*. The PR thread was not readable in this
run, so the following is a mechanism **consistent with** that summary and derived from HEAD, not a
reading of what the PR did.

`less_than_var:1254-1265` does not materialize a witness. It returns the *expression* `1 − q`:

```rust
let comparison = self.more_than_eq_var(lhs, rhs, bit_size)?;
let one = self.add_constant(F::one());
self.sub_var(one, comparison) // comparison negated
```

An `AcirVar` holding `AcirVarData::Expr` costs nothing until a consumer needs a `Witness`
(`get_or_create_witness_var:222-244`); linear expressions fold into whatever expression consumes them.
So on HEAD:

- `a >= b` and `a < b` are `q` and `1 − q` over **one** Brillig hint and **one** pair of range checks;
- `!(a < b)` is `not_var` on a `u1`, i.e. `1 − (1 − q)`, which folds back to `q` exactly
  (`not_var:767-778`) — zero additional opcodes;
- the `ASSERT w2 = -w3 + 1` line in the §2 snapshot appears *only* because `w2` is a return value and
  return values must be witnesses. In-circuit consumers pay nothing for the negation.

Any specialization that gives `<` its own lowering rather than deriving it from `>=` therefore risks
emitting a **second** hint plus a second pair of range checks for a program that uses both polarities,
and loses the free-negation identity — a regression whose size scales with how often comparisons are
used in both directions (very often, after `flatten_cfg` materializes both arms of every branch). That
is a plausible reading of "stopped sharing a witness", and it is the specific risk any future work in
this function must be measured against.

**Pre-flight for anyone re-opening this line of work:** read PR #10159 and its diff first. This record
could not, and the mechanism above is therefore a hypothesis.

## 6. What is actually left on the table (adjacent, NOT implemented, NOT measured)

§3c holds because `bound_constraint_with_offset`'s fast path fires. It is gated on

```rust
self.var_to_expression(rhs)?.to_const().and_then(|c| c.try_into_u128()).filter(|c| *c != 0)
```

(`:1061-1067`). At `rhs = 2^128` the `try_into_u128()` conversion fails — `2^128` is exactly one past
`u128::MAX` — so the fast path is skipped and the general path emits a fresh witness for
`rhs − (r + 1)` and range-checks *that*. Upstream's own snapshot for a 128-bit truncation
(`compiler/noirc_evaluator/src/acir/tests/instructions.rs:294-320`,
`truncate_field_to_128_bits`) shows it:

```text
BLACKBOX::RANGE input: w3, bits: 128                                   ← remainder: r < 2^128
ASSERT w4 = -w3 + 340282366920938463463374607431768211455              ← w4 = (2^128 − 1) − r
BLACKBOX::RANGE input: w4, bits: 128                                   ← second 128-bit range check
```

`RANGE(w3, 128)` already says `r < 2^128`, and the bound constraint says `r < rhs = 2^128` — the same
statement. But it is asserted about a *different* witness, so `redundant_range` cannot merge them, and
the circuit pays an extra 128-bit range gadget plus an `AssertZero` on every such call.

Reachability, both halves in this bead's area:

- **128-bit truncation** — `truncate_var:1165-1180` divides by `2^rhs`; at `rhs = 128` this is the
  snapshot above.
- **`u128` comparisons** — `acir/mod.rs:773-776` routes `BinaryOp::Lt` on
  `NumericType::Unsigned { bit_size }` to `less_than_var(lhs, rhs, bit_size)`, so a `u128` comparison
  reaches `more_than_eq_var(_, _, 128)`, whose divisor is `2^128`. `max_q_bits` is still `1` and
  `max_r_bits` is `128`, so the shape is §2's with the §6 gap added: one extra 128-bit range check and
  one extra `AssertZero` per `u128` comparison.

The minimal fix shape is a **one-condition change in `bound_constraint_with_offset`**: recognise a
constant `rhs` that does not fit `u128` (or, more narrowly, a constant `rhs` that is a power of two)
and take the padding path with the width computed from the field element rather than from a `u128`.
Nothing about the comparison lowering changes, so §5's witness-sharing risk is not engaged — the
returned `AcirVar` and every other opcode are untouched.

Constraint-equivalence argument for the removal (an argument, not a machine-checked proof — the
soundness burden here is that dropping a constraint must not *under*-constrain the circuit, which in a
proving system is a correctness bug, not a performance one): with `rhs = 2^k` constant,
`max_r_bits = (rhs − 1).num_bits() = k`, so the already-emitted `RANGE(r, k)` enforces `r ≤ 2^k − 1`,
which is `r < rhs` exactly. The bound constraint adds no accepting-set restriction. This must still be
demonstrated by test, not asserted — see §7.

**This is a candidate, not a result.** It is not implemented here, and per the measure-first mandate it
must not go upstream until §7 has been run.

## 7. Measurement protocol (required before any PR; not run here)

Needs a box with rustc `1.89.0`, a HEAD-built `nargo`, and a version-matched `bb` (via `bbup`).

1. **Reproduce the gap.** `cargo test -p noirc_evaluator truncate_field_to_128_bits` and add a
   `lt_u128` sibling to `acir/tests/instructions/binary.rs`; confirm the double 128-bit `RANGE`.
2. **Confirm the null result of §4 empirically** before spending effort elsewhere: build a fixture
   exercising `<`, `>=`, and both polarities of the same comparison at `u8`/`u32`/`u64`, and record
   `nargo info` opcodes and `bb gates -s ultra_honk` as the baseline. §4 predicts the sketch changes
   neither.
3. **Implement §6's one-condition change** on a branch; regenerate the affected ACIR snapshots and
   justify each diff line-by-line in the PR body.
4. **Differential execution** on `u128` comparison and 128-bit truncation fixtures, including boundary
   inputs (`0`, `2^128 − 1`, equal operands, `a = b ± 1`): `nargo execute --force` vs
   `nargo execute --force-brillig` must agree on witness and on failure behaviour. Add a negative test
   that a witness violating the removed constraint is still rejected.
5. **`bb gates -s ultra_honk` before/after** on those fixtures. A win must reproduce at gate level or
   the PR does not go up (§5.1, and the #10159 lesson).
6. **Corpus regression**: recompile the sparq corpus packages and the upstream `test_programs/benchmarks`
   set with the patched compiler; any unexplained increase anywhere is a stop signal.
7. **Compile-time sanity** on `sha512_100_bytes` (the #12927 O(n²) lesson).
8. Only then, upstream etiquette per `noir-optimization-program.md` §6 + §10.2.6 and `AGENTS.md`
   § *Upstream contributions*: draft PR, brevity, disclosure line, @jeswr review before ready.

## 8. Drafted upstream comment — NOT posted

To be posted only after §7 is run, and only with @jeswr's review. Kept to the brevity the maintainer
asked for; the numbers are placeholders precisely because they are not measured yet.

> Small ACIR-gen observation while looking at comparison lowering.
>
> `bound_constraint_with_offset` has a fast path for constant `rhs` that is gated on
> `try_into_u128()`. At `rhs = 2^128` that conversion fails, so the general path emits a fresh witness
> for `rhs - (lhs + offset)` and range-checks it — on top of the remainder range check that already
> says the same thing. It is visible in the committed snapshot for `truncate_field_to_128_bits`: two
> 128-bit `RANGE` opcodes where one suffices. The same shape shows up on `u128` comparisons, which
> reach `more_than_eq_var` with divisor `2^128`.
>
> For a constant power-of-two `rhs`, `RANGE(r, k)` with `k = (rhs-1).num_bits()` already enforces
> `r < rhs`, so the bound constraint adds nothing. Recognising that case in the fast path removes one
> 128-bit range gadget and one `AssertZero` per site. Measured: `<gates before → after>` on
> `<fixture>`; corpus and benchmark projects `<result>`.
>
> Not proposing anything for `more_than_eq_var` itself — the "quotient is 0/1, so specialize the
> split" idea turns out to be already realized by the constant-`rhs` bound tightening
> (`max_q_bits = 1`), so a specialized path is opcode-identical, and #10159 is the reminder that
> touching that function has a downside and no upside here.
>
> Developed with support from generative AI; author review before maintainer review.

## 9. Outcome and follow-ups

- **Sketch (§7 row 9 as written): rejected on the evidence.** Opcode-identical to HEAD; no PR. This
  is the acceptable null result the bead permits, and it should be recorded in
  `noir-optimization-program.md` §4.2 alongside #10159 so it is not re-derived.
- **`sq-jfkwk` stays open**, blocked on the same empirical half as `sq-eesz3` (§10.5) and `sq-felqr`
  (§10.6) — no rustc `1.89.0`, no `nargo`, no `bb` on this box — and, for §5, on reading the #10159
  thread.
- **One adjacent candidate** (§6) is specified and left unimplemented, with §7 as its gate. It is
  narrower than the bead's sketch, sits in a different function, and does not engage the
  witness-sharing risk.
- The recurring blocker across all three noir beads is environmental: the empirical half of this
  program needs a box pinned to noir's `rust-toolchain.toml` with `nargo` and `bb` installed.
