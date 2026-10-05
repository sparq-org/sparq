# Truncate after `and`-mask → bound from the mask (noir#8628 / noir#13264) — verification record

> 🤖 **SPARQ agent** [OPUS-5] — bead `sq-9xhoa`, row 2 of
> `noir-optimization-program.md` §7, epic `sq-uuvac`.
> **Outcome: already implemented upstream as draft noir#13264 — do NOT
> re-implement.** This record verifies that PR's load-bearing claims against noir
> `master` (`d89d99a9`, read 2026-08-01) and lists what still blocks the
> draft → ready flip. **Nothing was measured or executed**: this session had no
> `nargo`, no `bb`, and no built compiler — only a blobless sparse checkout of
> `compiler/noirc_evaluator` at that SHA plus the PR's raw diff. Every statement
> below is source-derived or diff-derived and is labelled as such.

## 1. Tracking status

`sq-9xhoa` was re-dispatched as an implementation task. It is not one. The row-2
optimization was written and shipped as **draft noir#13264** (3 commits, dated
2026-07-04) in steps 1–2 of the program's §8 sequencing, and §10.1 already lists
it; the bead is open because the §6 protocol gate — @jeswr's author review — has
not been passed, not because code is missing. This is the same shape as §10.11
(`sq-b0vpc` / noir#13314) and §10.12 (`sq-fwcuo` / noir#13265).

Verified this session, without the GitHub API (raw/static fetches + a
`--filter=blob:none --sparse` clone only):

| check | result |
|---|---|
| does `master`'s `remove_truncate_after_range_check.rs` handle `and`? | **no** — its `match` has exactly three arms (`RangeCheck`, `Truncate`, `_ => ()`) ⇒ the PR is unmerged |
| does noir#13264's diff still apply to `master`? | **yes** — `patch -p1 --dry-run` applies all 3 hunks of the pass file with **no offset and no fuzz**, plus the 5 new files |
| rebase hazard | **absent, unlike #13265**: the diff touches **no existing snapshot file**. It is the pass + its unit tests + one new `execution_success` fixture and that fixture's two new `nargo_cli` snapshots |
| every API the diff uses exists at `master` | `instruction_result::<N>() -> [ValueId; N]` (`dfg.rs:699`), `get_numeric_constant() -> Option<FieldElement>` (`dfg.rs:741`), `Binary`/`BinaryOp` re-exported at `ssa::ir::instruction` (`instruction.rs:25`), and `use acvm::AcirField;` is the existing `ssa/opt` idiom (`array_set.rs:61`, `make_constrain_not_equal.rs:30`) |

## 2. What the PR does

It adds one `match` arm to the pass: on `Binary { operator: And }` where **exactly
one** operand is a numeric constant, it records `mask.num_bits()` as a bound on the
instruction's result, into the same `range_checks: HashMap<ValueId, u32>` the
`RangeCheck` arm already feeds, with the same `and_modify`-takes-the-minimum
discipline. The existing `Truncate` arm then removes any truncation whose
`bit_size` is at least that bound. A fully-constant `and` is declined (folded
elsewhere); a non-constant mask is declined (no bound).

## 3. Equivalence — what is verified, and against what

| claim | verified against | status |
|---|---|---|
| `num_bits()` is the **value's** bit length (highest set bit + 1), not a type width, so `123 → 7` | `acir_field/src/field_element.rs:414-423` returns `i*64 + (64 - limb.leading_zeros())` for the top non-zero limb, `0` for zero; upstream proptest `num_bits_agrees_with_ilog2` (`:658-661`) pins `num_bits == ilog2 + 1` | **holds** |
| the bound itself: every bit set in `x & m` is also set in `m`, so `x & m <= m < 2^num_bits(m)` | arithmetic, over the integer representation `BinaryOp::And` operates on — it evaluates as `\|x, y\| Some(x & y)` (`instruction/binary.rs:382`). The argument needs no type precondition: the inequality holds for any non-negative representation. (Upstream additionally documents bitwise ops as unsupported for `Field` — the comment and the absent folding function at `instruction/binary.rs:365-367` — but the bound does not rest on that) | **holds** |
| **the dominance question is vacuous for this arm** | the bound is attached to the `and`'s *own result* — a fresh SSA value. SSA guarantees a definition dominates all its uses, so any `truncate` consuming that result is necessarily dominated by the `and`. Unlike the `RangeCheck` arm (where the checked value may be defined far from the check, which is what the `previous_block`/`dominates` heuristic at `:40-49` approximates), this arm needs no dominance reasoning for validity | **holds** — the clearing heuristic can only cost a *missed* rewrite here, never an unsound one |
| taking the **minimum** when a value already has a bound | both bounds hold simultaneously, so their min holds | **holds** |
| mask `0` cannot reach the arm | `simplify/binary.rs:236-239` folds `and` with a zero operand to the constant zero before the pass | **holds** (so `num_bits(0) == 0` is unreachable, not merely benign) |
| the fixture comment's claim that masks of the form `2^n - 1` never reach the pass as an `and` | `simplify/binary.rs:243-273`: for `is_unsigned()` types, an `and` by a constant `bitmask` with `(bitmask + 1).is_power_of_two()` (or `u128::MAX`) is rewritten to `Instruction::Truncate`, or elided outright at full type width | **holds, but only for unsigned** — see §4 |
| all seven masks the PR tests with are genuinely not `2^n - 1` | recomputed: `123`, `0xab`, `0x1234`, `0x1ab`, `5`, `507`, `0x8000000000000075` — none satisfies `(m+1) & m == 0` | **holds** |
| the fixture's four expected values | recomputed from `x = 0x123456789abcddf5`, `y = 0xcafebabe`: `a = x & 123 = 113` (mask 7 bits ≤ 8, removable); `b = x & 0xab = 161` (8 ≤ 8, removable); `c = y & 0x1234 = 4660` (13 ≤ 16, removable); `kept = (x & 0x1ab) % 2^8 = 161` (mask 9 bits > 8, **not** removable) | **all four match the committed `Prover.toml`** |

Per `sq-qhy4` this is **not** a soundness certification: it is a source-level
argument that the rewrite preserves the constraint set on the paths read above,
unbacked by execution, by gate measurement, or by external review.

### 3.1 Are the tests non-vacuous? (source-derived; **not executed**)

The PR is better guarded than #13265 was. Reading the shipped tests against the
mutations they are supposed to catch:

- delete the whole `and` arm → the two positive snapshot tests
  (`removes_truncate_after_and_with_constant_mask`,
  `..._non_contiguous_mask_up_to_highest_set_bit`) go red. The headline guard is
  **not** vacuous.
- compute the bound as the mask's **popcount** instead of its bit length →
  `does_not_remove_truncate_below_the_masks_highest_set_bit` (mask `5 = 0b101`,
  truncate to 2 bits) goes red. This is the sharpest guard in the diff and it is
  the exact mistake the optimization invites.
- flip the `<=` comparison direction →
  `does_not_remove_truncate_after_and_with_mask_wider_than_truncation`
  (`507`, 9 bits, truncate to 8) goes red.
- at execution level, the fixture's `kept` case is live: if the rule over-fired
  there the value would be `417` rather than `161`, so `assert(kept ==
  expected_kept)` fails. Note the deliberate trap that `expected_kept ==
  expected_b == 161` — the fixture author chose an input where the *masked* and
  *truncated* results differ, which is what makes the assertion bite.

## 4. The one real gap: signed types

`simplify/binary.rs:243` guards the `2^n - 1` → `Truncate` canonicalization with
`lhs_type.is_unsigned()`, and `NumericType::Signed` exists as a distinct variant
(`ir/types.rs:25-29`, `is_unsigned()` at `:91-93` matches only `Unsigned`).
Consequently:

> On a **signed** integer type, an `and` by a `2^n - 1` mask is *not*
> canonicalized away, so it reaches the pass as an `and` and the new arm fires on
> it — a mask shape the arm can **never** see on an unsigned type.

Every one of the PR's seven unit tests, and all four fixture cases, are unsigned
(`u64`/`u32`). So the single mask class the new rule uniquely reaches on signed
types has **no coverage at all**. That signed truncations genuinely exist in the
IR is confirmed independently by `acir/mod.rs:872-880`, which carries a dedicated
`unreachable!("Truncation of unchecked signed subtraction")` for a signed operand
of a `Truncate`.

The rewrite still *looks* correct there — noir represents signed integers in
two's complement over `[0, 2^bits)`, `and` is bitwise on that representation
(so the result is still `<= mask`), and `Truncate` is `% 2^bit_size` on that same
representation — but "looks correct, wholly untested, on the one type class the
rule uniquely reaches" is precisely the finding a reviewer should force closed.
**Not executed here**; the concrete check is whether the frontend emits an
`and`-then-`truncate` shape for a signed narrowing cast.

## 5. Placement: the pass, or `get_value_max_num_bits`?

Neither existing mechanism covers this case today, so the PR is **not** redundant —
verified:

- `dfg.get_value_max_num_bits` (`dfg.rs:574-634`) has arms for `Cast` and for
  unchecked ACIR `Add`/`Sub`/`Mul` only; every other `Binary` falls to
  `_ => value_bit_size`, i.e. the static type width. It does not know that
  `x & 123` fits in 7 bits.
- the `Truncate` rule in `simplify.rs:210-262` has arms for `bit_size >=
  max_bit_size`, a constant value, a nested `Truncate`, and — structurally the
  closest precedent — `Binary { operator: Div }` **by a constant**, whose comment
  reasons about the quotient's max bit size exactly as this PR reasons about a
  mask. There is no `And` arm.

That precedent is the design question a maintainer may raise. Teaching
`get_value_max_num_bits` (or `simplify.rs`'s `Truncate` arm) about `and`-with-a-
constant instead of, or as well as, the pass would be a comparably small diff
and strictly more general, because:

1. simplification runs at instruction-insertion time, everywhere, rather than at
   the single pipeline position `ssa/mod.rs:395`; and
2. `get_value_max_num_bits` also backs the `RangeCheck` removal at
   `simplify.rs:281-284` (`max_potential_bits <= max_bit_size ⇒ Remove`), so an
   `And` arm there would drop redundant **range checks** on masked values for
   free — a win the PR's truncate-only rule does not get.

Against that: jfecher's in-issue direction was explicitly *"we should probably add
tracking for `and` there"*, i.e. in this pass, so the PR follows the maintainer's
stated path. Recording the alternative as a **question to raise on the PR**, not
as a defect. It also connects directly to row 7 (`sq-jj3ne`, value-range/fact
tracking), which is where a general bound lattice belongs.

## 6. Cost model (structural, not measured)

The bead's stated win holds structurally: ACIR-gen lowers `Instruction::Truncate`
(`acir/mod.rs:519`) through `convert_ssa_truncate` (`:856-892`) to
`acir_context::truncate_var` (`acir_context/mod.rs:1165-1179`), which is a
`euclidean_division_var` by `2^bit_size` — the hint-plus-range-checks lowering,
not a cheap operation. Crucially, `convert_ssa_truncate` has **no early-out** on
the value's known bit width: it always emits the division. So removing the
truncate in SSA is the only thing that avoids that cost, and each removal avoids
a whole lowering.

No opcode delta, no gate count, and no compile-time figure were produced here.
Per §5.1 `bb` gates are the arbiter, and per §4.2's #10159 lesson opcode counts
alone mislead — so the size of the win remains **unquantified** by this bead.

## 7. Frequency — does it pay rent?

Weak evidence, stated as weak: the diff updates **no existing snapshot**, so on
the in-tree corpus the new rule apparently changes nothing that any committed
snapshot observes. That is a much softer signal than it looks — the SSA snapshots
cover specific passes, and the `nargo_cli` snapshots capture expanded source and
stdout rather than ACIR — so it is consistent with both "the shape is rare
in-tree" and "in-tree tests simply do not observe it". Contrast #13265, whose rule
demonstrably fired on compiler-generated IR (§10.12). **Corpus frequency was not
measured here**, and this is again the PR #11580 "does it pay rent" question.

One structural reason to expect the shape to be *less* common than it first
appears: the most idiomatic masks users write (`& 0xFF`, `& 0xFFFF`) are exactly
the `2^n - 1` masks that `simplify/binary.rs` already turns into truncations, so
they never reach this arm on unsigned types. The arm's reachable population on
unsigned types is the *non*-`2^n - 1` masks, which is why the PR's own tests all
have to use unusual constants.

## 8. Preconditions before the draft → ready flip

| finding | why it blocks |
|---|---|
| **No signed-type coverage (§4)** — and signed is the only place a `2^n - 1` mask reaches the rule | the one uniquely-reachable case has no test; a few SSA unit tests over `i32`/`i64` close it, and are cheap to land on the PR branch |
| **No measurement, and no fixture that would ever show one** — the fixture lands in `execution_success`, not `test_programs/benchmarks/`, so no committed number moves when this rule regresses; this record adds no numbers either | §5.1 makes `bb` gates the arbiter and §10.2 steps 3–4 require the corpus + gate runs. Blocked on `sq-i50o4` (a `bb` binary in the measurement environment), which §10.10 already names the program's highest-value next action. Same gap class as §10.12 |
| **The placement question (§5) is unaddressed in the PR body** | a reviewer who notices the `Div`-by-constant precedent in `simplify.rs` will ask why this went in the pass; the answer (jfecher's in-issue direction) is good but should be *stated*, ideally with the `RangeCheck`-removal bonus noted as follow-up work |
| **A conservatism the PR documents but does not fix**: its own `does_not_remove_truncate_in_block_not_dominated_by_the_previous_block` test pins a case where `b0` *does* dominate `b2` and the truncate is therefore genuinely removable, but the `previous_block` clearing heuristic drops the bound anyway | correct-but-conservative, so not a correctness blocker. Per §3 the map could safely retain its `and`-derived entries across that clear, since those bounds are valid at every use by SSA dominance. Worth one sentence in the PR body so a reviewer does not read the test as an admission of unsoundness |

## 9. Recommended disposition

`sq-9xhoa` stays **open**, blocked on the same two things as the rest of the
program: the §6 author review and the missing measurement environment. The
implementation work is done and is in better shape than row 3's — its diff still
applies cleanly, it carries genuine red-on-wrong-answer guards (§3.1), and it
touches no snapshot that could rot. The remaining work is the signed-type tests
(§8 row 1, small, worth landing on the PR branch), then the §10.2 measurement
protocol. No upstream comment was drafted or posted from this session, and the
PR's own body and review thread were **not read** — only its diff.
