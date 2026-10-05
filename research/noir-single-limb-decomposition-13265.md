# Single-limb `to_bits`/`to_radix` → range constraint (noir#13265) — verification record

> 🤖 **SPARQ agent** [OPUS-5] — bead `sq-fwcuo`, row 3 of
> `noir-optimization-program.md` §7, epic `sq-uuvac`.
> **Outcome: already implemented upstream as draft noir#13265 — do NOT
> re-implement.** This record verifies that PR's load-bearing claims against noir
> `master` (`d89d99a9`, read 2026-07-31) and lists what still blocks the
> draft → ready flip. **Nothing was measured**: this session had no noir checkout,
> no `nargo` and no `bb`; every statement below is source-derived or diff-derived
> and is labelled as such.

## 1. Tracking status

`sq-fwcuo` was re-dispatched as an implementation task. It is not one. The row-3
optimization was written and shipped as **draft noir#13265** in steps 1–2 of the
program's §8 sequencing, and §10.1 already lists it; the bead is open because the
§6 protocol gate — @jeswr's author review — has not been passed, not because code
is missing. This is the same shape as §10.11 (`sq-b0vpc` / noir#13314).

Verified this session, without the GitHub API (raw/static fetches only):

| check | result |
|---|---|
| the two TODOs the bead targets — `ssa/ir/dfg/simplify/call.rs:59` and `:79`, *"simplify to a range constraint if `limb_count == 1`"* | **still present at `master`** ⇒ the PR is unmerged |
| noir#13265's diff | 8 files: `simplify/call.rs` (the rule + 9 unit tests), snapshot updates in `constant_folding/mod.rs` and `remove_bit_shifts.rs`, and a new `test_programs/execution_success/to_radix_single_limb` fixture + its two `nargo_cli` snapshots |
| does the `call.rs` half still apply to `master`? | **yes** — `patch --dry-run` applies all five hunks (one 1-line offset). Only that file was checked; the two touched *snapshot* files are the usual rebase hazard and were not |

## 2. What the PR does

`simplify_single_limb_decomposition` replaces the intrinsic call with
`RangeCheck { value, max_bit_size: log2(radix) }` followed by `Cast(value,
limb_type)` and a one-element `MakeArray`. It declines unless the return type is
`[T; 1]`, the element type is numeric, and the radix is a **constant power of two
in `2..=256`**. `ToBits` passes radix 2 (⇒ a 1-bit check); `ToRadix` reads the
radix from `dfg.get_numeric_constant(arguments[1])`. Endianness is irrelevant for
one limb, and the constant-value case still takes the pre-existing constant-fold
path. Inserting instructions from inside a simplification is the file's existing
idiom (the vector helpers at `call.rs:805`/`:836`/`:847` do it).

## 3. Equivalence — what is verified, and against what

| claim | verified against | status |
|---|---|---|
| a single-limb decomposition constrains exactly `value < radix` and returns `[value]` | `acir_context/generated_acir/mod.rs:381-429`: `radix_le_decompose` emits the hint witnesses, one `range_constraint(limb, bit_size)` each, and one `assert_is_zero(input − Σ limbᵢ·radixⁱ)`. At `limb_count == 1` that is `RANGE(w, bit_size)` + `w == input` | **holds** |
| the reused failure message `"Field failed to decompose into specified 1 limbs"` is the one both runtimes already produce | ACIR: `generated_acir/mod.rs:422-424`. Brillig: the ACVM black box `brillig_vm/src/black_box.rs:345-349` returns `AssertFailed` with the **same** `format!` string | **holds, in both runtimes** |
| the power-of-two restriction costs nothing in ACIR | `acir_context/mod.rs:1285-1288` — `radix_decompose` itself `assert!`s `radix.is_power_of_two()`, mirroring `Field::to_le_radix`. The declined non-power-of-two / non-constant radix cases are **Brillig-only**, which is exactly how the PR's two decline tests are written | **holds** |
| the cast adds no constraints | `acir/mod.rs:504-507` — `Instruction::Cast` is `convert_numeric_value` + `define_result_var`, i.e. a pure alias in ACIR. The returned limb *is* the range-checked input | **holds** |
| under a predicate the rewrite stays equivalent | `flatten_cfg.rs:1472-1478` rewrites `RangeCheck`'s operand to `value * condition` **and** calls `predicate_value(value, predicate_value)`, which `map_value`s the original id (issue #8617) — so the following `Cast` resolves to the predicated value too. The un-rewritten call was predicated the same way, by zeroing its argument in `handle_call_side_effects` (visible in the `constant_folding` snapshot the PR updates: `v10 = mul v0, v9; v11 = call to_be_radix(v10, …)`) | **holds** |

Per `sq-qhy4` this is **not** a soundness certification: it is a source-level
argument that the rewrite preserves the constraint set on the paths read above,
unbacked by execution, by gate measurement, or by external review.

## 4. Cost model (structural, not measured)

Per firing site the rewrite removes the `brillig_to_radix` hint opcode, the fresh
limb witness, and the recomposition `AssertZero` + its assertion payload; the
range check survives but moves from the hint witness onto the input. **Caveat that
must be respected when this is finally measured:** the removed `AssertZero` is a
two-term equality, precisely the shape the ACVM `MergeExpressionsOptimizer`
refunds (cf. §10.6), so the delta counted from SSA/ACIR-gen is an **upper bound**
on the post-ACVM delta. Count opcodes after ACVM transforms, and treat `bb gates`
as the arbiter (§5.1, the #10159 lesson).

## 5. Frequency — does it pay rent?

One firing site is **compiler-generated**, not user-written: `remove_bit_shifts.rs`
lowers a dynamic shift by decomposing the exponent, and special-cases an exponent
whose `get_value_max_num_bits` is 1 to `max_exponent_bits = 1`
(`remove_bit_shifts.rs:347-363`), emitting `call to_le_bits(v) -> [u1; 1]`. Its
committed snapshot (`:698`) is exactly that shape, and the PR's update to it
replaces the call and its `array_get` with a `make_array` of the original `u1`
value — with **no `range_check` surviving at all**, because
`simplify.rs:281-284` removes a range check whose value's
`get_value_max_num_bits` already fits. So at that site the decomposition is
removed outright and nothing is emitted in its place. This mirrors §10.10's
finding for #13263 (the rule fires on compiler-generated `Div`s). **Corpus
frequency was not measured here** — that remains open, and it is the PR #11580
"does it pay rent" question.

## 6. Preconditions before the draft → ready flip

| finding | why it blocks |
|---|---|
| **The failure path has no live test.** The fixture's out-of-range input (`y`) is only decomposed under `if cond` with `cond = false`, which tests the *predicated* path. Nothing pins that a single-limb decomposition of a value ≥ radix still fails, with the same message, on a taken path — in either runtime. That is the red-on-wrong-answer guard for the whole rewrite: delete the emitted `RangeCheck` and every shipped test still passes | an `execution_failure` fixture (or a `should_fail_with` test) is a few lines and closes it. Same gap class as §10.11's first finding |
| **No measurement, and no fixture that would ever show one.** The PR adds a fixture under `execution_success`, not under `test_programs/benchmarks/`, so no committed number moves when this rule regresses. This record adds no numbers either — no ACIR opcode delta, no gate count, no corpus recompile | §5.1 makes `bb` gates the arbiter and §10.2 steps 3–4 require the corpus + gate runs. Blocked on `sq-i50o4` (a `bb` binary in the measurement environment), which §10.10 already names the program's highest-value next action |
| **The Brillig `RangeCheck` message path is unverified.** §3 establishes the ACVM black box produces the same string; it does not establish that Brillig codegen for `Instruction::RangeCheck` surfaces the `assert_message` rather than a generic trap | if it does not, `--force-brillig` failure *messages* diverge even though failure *behaviour* matches — cheap to check, and §10.2 step 2's differential is where it shows up |

## 7. Recommended disposition

`sq-fwcuo` stays **open**, blocked on the same two things as the rest of the
program: the §6 author review and the missing measurement environment. The
implementation work is done; the remaining work is the failure-path test (§6 row 1,
small and worth landing on the PR branch), then the §10.2 measurement protocol.
No upstream comment was drafted or posted from this session, and the PR's own body
and review thread were **not read** — only its diff.
