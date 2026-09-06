# Noir optimization — gap analysis over the un-examined compiler stages (`sq-mtolx`)

<!-- [OPUS-5] sq-mtolx (#3127): P3 of the Noir-optimisation paper program (parent
     bead sq-1j5ow, epic sq-uuvac). DESIGN-FOR-REVIEW / DOC-ONLY — no compiler
     source is modified, no upstream PR is opened, and NO bead is spawned (the
     spawn gate is unmet; see §7). Produces the paper's "new opportunities"
     section (§6) from a code-level read of noir-lang/noir @ 8f33502e. -->

> 🤖 **SPARQ agent** — gap-analysis record for bead `sq-mtolx`, child of the paper
> bead `sq-1j5ow` (Noir upstream optimization program, epic `sq-uuvac`).
> Model: Opus 5. This is a **design-for-review** record for the maintainer
> (@jeswr). It reads code; it measures nothing; it opens no upstream PR and
> spawns no bead.

## 0. Brief corrections (read first — honesty)

The activating brief (issue #3127) sets four premises. Three are wrong against
the code and the program's own prior records, and the corrections change what
this bead could deliver.

1. **"For EACH candidate that reproduces a bb-gates win via the P1 harness" —
   the P1 harness does not exist, and no candidate can be measured.** Bead
   `sq-i50o4` (P1, commit a reproducible measurement harness) has **not landed**:
   there is no committed Noir measurement harness anywhere in this repo
   (`bench/zk-compose/scripts/bb_gates_matrix.py` measures *sparq's own*
   `zk/compose` circuits from a regression-gated snapshot; it does not build or
   measure the Noir compiler). The executing box for this bead has no Noir
   checkout, no `rustc`/`nargo`, and no `bb`. Worse for the premise: the
   program's own prior runs establish that **`bb` was absent even on the Noir
   work box** — `noir-optimization-program.md` §10.8 and §10.9 both record
   *"`bb` is still absent, so gate counts remain out of reach"*, in the two runs
   that *did* successfully build the compiler. So the gate-measurement
   capability the brief assumes has never existed in this program.
   **Consequence: the spawn condition is unmet by construction, not by candidate
   failure. Zero beads are spawned (§7).**
2. **"the passes NOT yet implementation-examined" — the brief's list includes
   three that have already been examined, two of them to a null result.** ACIR-gen
   comparison lowering (`acir_context/mod.rs:1163-1228`, the #10159 landmine) was
   examined by `sq-jfkwk` and **returned a null result** — the sketch is rejected
   on the evidence (`noir-comparison-lowering-10159.md`, program §10.7). Quadratic
   ACIR expression growth (#4629) was examined by `sq-felqr` and produced a
   **design note** (`noir-acir-expression-growth-4629.md`, program §10.6). The
   array/memory merging item (#5501) is **already beaded** as `sq-m3l62` and
   assigned in the fleet spec (program §10.3). This record therefore re-verifies
   the two null results at the new pin (§5.7) rather than re-deriving them, and
   treats only the genuinely un-examined passes as new ground.
3. **"ARRAY/MEMORY merging … `flatten_cfg/value_merger.rs`
   `try_merge_only_changed_indices` #5501" — that function no longer exists, and
   the optimisation it names is already implemented elsewhere.** See §5.3: the
   file moved to `ssa/ir/dfg/simplify/value_merger.rs`, the named function was
   **removed upstream by PR #8142**, the live upstream tracker is **#8145 not
   #5501**, and the "merge only the changed indices" ask is implemented in
   `flatten_cfg::try_optimize_array_set_merge`. Bead `sq-m3l62`'s target as
   written in program §10.3 points at a deleted function in a moved file and
   **must be re-spec'd before any fleet agent picks it up** (§8).
4. **The brief's framing "produce measured candidates" cannot be honoured; what
   this bead produces is code-level candidate *specs* with explicit measurement
   recipes.** Every candidate below is an **unmeasured hypothesis**. No gate
   count, no ACIR opcode count, and no compile-time figure in this record was
   produced by running anything.

One framing the record carries throughout, unchanged from the paper skeleton:
these are **circuit-size (proving-cost) optimisations**. They neither change nor
make any claim about the *cryptographic* privacy or proof-system soundness of the
ZK backend. That is not a blanket soundness disclaimer: a compiler rewrite is
**constraint-soundness-sensitive** — getting one wrong under-constrains the
circuit, so it accepts witnesses the original rejected — and several candidates
below are explicitly of that shape (P3-5 carries the highest under-constraining
risk in the register, P3-4 may have been removed upstream *for* soundness, and
the P3-1 constraint-width change can admit values outside the caller's budget).
Each candidate therefore carries its own equivalence / fail-closed obligation,
stated with it, and that obligation is load-bearing.

## 1. What was actually done

- **Pin.** noir-lang/noir at commit `8f33502e78e10eb84867b0c009eaa9e463b76ce4`
  (fetched 2026-07-29), read as a shallow sparse checkout of
  `compiler/noirc_evaluator` + `acvm-repo` outside this repo. Prior program
  records analysed `0df14918`, `e22cd89b` and `a352e715`; because the checkout is
  shallow, **no drift could be diffed** — every "moved / renamed / removed"
  statement below is a description of the current tree, corroborated where
  possible by an in-tree comment, never by `git log`.
- **Method.** Seven parallel code-level examinations, one per un-examined stage,
  each instructed to try to reach a *null* verdict first (the program's dominant
  prior outcome), to treat a stated in-code rationale as evidence of deliberate
  design rather than an opportunity, and to name the pass it checked before
  claiming a gap is unabsorbed.
- **Estate verification at this pin (done directly, not delegated).** None of the
  program's four fresh draft PRs has landed: the `BinaryOp::Div` arm of
  `ssa/ir/dfg/simplify/binary.rs:144-159` still carries only the `rhs == 1` and
  `NativeField`-inverse rules (no #13263 rule); the two `to_bits`/`to_radix`
  TODOs are still present verbatim at `ssa/ir/dfg/simplify/call.rs:59,79`
  (#13265 unmerged); and there is no `ssa/opt/remove_redundant_range_checks.rs`
  (#13266 unmerged). `remove_truncate_after_range_check.rs` exists as the
  pre-existing pass #13264 extends.
- **Not done.** No build, no `nargo`, no `bb`, no benchmark, no upstream issue or
  PR thread was read (no GitHub API calls were made from this bead). Upstream
  issue numbers are carried **as recorded in prior program records, not
  re-verified** — several may be closed or stale, and #5501 demonstrably is
  (§0.3). Barretenberg source is not in the checkout, so every statement about
  what a construct costs *in gates* is inference from the ACIR the compiler
  emits, not from the backend.

## 2. The structural finding (the paper's real contribution here)

The dominant result of this pass is **not** a list of wins. It is a structural
account of *why* the classical compiler-optimisation catalogue is largely null in
this cost model. Three absorption layers stand between an SSA-level pessimism and
the gate count, and each one independently voids a whole family of candidates:

1. **Mandatory full unrolling.** ACIR loops are always fully unrolled or the
   compile fails. The frontend rejects `while`, `loop`, `break` and `continue` in
   constrained code (`noirc_frontend/src/hir/resolution/errors.rs:114-123`); an
   unknown-bound `for` is a hard `RuntimeError::UnknownLoopBound`
   (`ssa/opt/unrolling.rs:389-432`); and `checks.rs:80-88` `assert_no_loops`
   panics if any loop survives. Unrolling substitutes the induction variable with
   a literal (`unrolling.rs:1305-1311`) through an inserter that simplifies at
   insertion time (`ir/function_inserter.rs:145`). **Every loop-level transform
   must justify itself on the unrolled straight-line code**, which kills loop
   unswitching and induction-variable strength reduction outright (§5.2).
2. **Post-unroll SSA CSE.** `fold_constants_using_constraints` and
   `fold_constants` run repeatedly after unrolling (`ssa/mod.rs:349,355,398,402`)
   with a dominance-aware, predicate-keyed cache that treats `Cast`/`Not`/
   `Truncate` as always-deduplicable and `Binary`/`ArrayGet` as deduplicable under
   the same predicate (`ssa/opt/constant_folding/mod.rs:868-909`,
   `result_cache.rs:97-105`). After unrolling, N copies of a *loop-invariant*
   instruction are literally the same instruction under the same predicate — so
   **a LICM hoisting miss on an invariant instruction is, for gate count, very
   likely a no-op** (§5.1).
3. **ACVM-level CSE and intermediate refunding.** After ACIR generation,
   `acvm::compiler::optimize` (`compiler/noirc_evaluator/src/ssa/mod.rs:746-748`)
   runs a circuit-global, scale-invariant common-subexpression cache
   (`acvm-repo/acvm/src/compiler/optimizers/common_subexpression/mod.rs:181`,
   `csat.rs:267-307`) and a `MergeExpressionsOptimizer` that eliminates by
   Gaussian elimination *any* non-IO witness used in exactly two `AssertZero`
   opcodes (`merge_expressions.rs:59-170`). **This discounts the entire
   "don't materialise this temporary" and "share this repeated subexpression"
   family of SSA-level candidates** — and, symmetrically, means an SSA pass that
   *creates* a two-use temporary will have it silently undone.

A fourth, sharper observation belongs with them: **the ACVM layer's own success
metric is ACIR opcode count** — its transform loop terminates on
`new_opcode_count == prev_opcode_count`
(`common_subexpression/mod.rs:135`, rationale at `:112-115`) and is capped at
three passes with a checked-in circuit that provably exits on the cap, not on
convergence (`mod.rs:645-647`). The layer immediately upstream of the backend is
tuned on the metric the program's own methodology says is not proving cost. That
is the #10159 trap institutionalised, and it is a genuinely publishable
observation.

Consequence for the paper: the honest headline of the "new opportunities" work is
that **the surviving candidates are concentrated where none of the three
absorption layers reaches** — expensive *lowering formulations* (a euclidean
division emitted where a cheaper equivalent exists), and *N-distinct-constraints
→ 1* rewrites that CSE structurally cannot perform because the N copies differ.

## 3. Coverage map — what was examined

| Stage | Sites read (at pin `8f33502e`) | Outcome |
|---|---|---|
| Loops — LICM | `ssa/opt/loop_invariant.rs` + `loop_invariant/simplify.rs`, driven from `ssa/mod.rs:284`, `preprocess_fns.rs:58` | 1 candidate (**P3-3**); 4 pessimisms absorbed by post-unroll CSE |
| Loops — unrolling | `ssa/opt/unrolling.rs`, `ssa/mod.rs:290-299,360-368` | **null** — both named upstream signals moot under full unrolling |
| Array/memory merging | `ssa/ir/dfg/simplify/value_merger.rs`, `ssa/opt/flatten_cfg.rs`, `array_set_window_optimization.rs`, `mutable_array_set.rs`, `remove_if_else.rs` | 2 candidates (**P3-4**, **P3-5**); the #5501 ask itself **already handled** |
| Memory SSA | `ssa/opt/mem2reg.rs`, `load_store_forwarding.rs`, `alias_analysis.rs`, `rc.rs`, `die.rs` | **null — not proof-cost-relevant** |
| Conditionals | `ssa/opt/basic_conditional.rs` | **null — Brillig-only** |
| Signed / wide lowering | `expand_signed_math.rs`, `expand_signed_checks.rs`, `check_u128_mul_overflow.rs`, `remove_bit_shifts.rs` | 1 candidate (**P3-2**) + a cost table (§5.5) + 1 corroboration of #13263 |
| ACIR-gen | `acir/acir_context/mod.rs` comparison + `bound_constraint_with_offset` | 1 candidate (**P3-1**); prior null result **re-confirmed** |
| ACVM | `optimizers/general.rs`, `common_subexpression/`, `redundant_range.rs`, `merge_expressions.rs` | **null / gates-risk-inverted**; produced the §2.3 absorption inventory |

## 4. Candidate register (all UNMEASURED hypotheses)

Ranked by (plausibility of a real gate win) × (breadth of firing shape) ÷ (risk).
**None of these is a claimed win.** Each is a code-grounded hypothesis with a
stated way it could be null.

### P3-1 — `bound_constraint_with_offset` charges 128-bit operations a redundant range check

- **Site.** `compiler/noirc_evaluator/src/acir/acir_context/mod.rs:1019-1109`,
  with the deciding gate at `:1061-1066`.
- **Current behaviour.** The constant-`rhs` fast path is entered only when
  `to_const().and_then(|c| c.try_into_u128())` succeeds. `try_into_u128` is
  `fits_in_u128().then(...)` where `fits_in_u128` is `num_bits() <= 128`
  (`acvm-repo/acir_field/src/field_element.rs:315-317,441-443`), so **every
  constant `>= 2^128` falls through to the general path**, which materialises a
  fresh witness, an `AssertZero`, and a second full-width range check.
- **Who pays.** Tracing every constant-`rhs` caller, exactly two producers pass
  `2^128`: `truncate_var(lhs, 128, max_bit_size)` (`mod.rs:1165-1179`), i.e.
  `Field as u128`; and `more_than_eq_var(lhs, rhs, 128)` (`mod.rs:1216`), i.e.
  **every `u128` `<` / `>` / `<=` / `>=`**. The 8-bit and 64-bit sibling paths pay
  nothing, because their fast path produces a check on the *same* witness that
  the ACVM `RangeOptimizer` then removes.
- **Evidence it is real.** Upstream's own committed post-optimizer fixture
  `truncate_field_to_128_bits` (`acir/tests/instructions.rs:294-345`) shows the
  extra `AssertZero` + second 128-bit `RANGE` on a fresh witness; the 64-bit
  sibling (`instructions.rs:243-292`) shows the same position emitting nothing.
  A second fixture pins the general-path output directly
  (`acir_context/mod.rs:1722-1769`).
- **Proposed rewrite.** Compute the fast path in field/`BigUint` arithmetic
  instead of `u128`, **plus** a guard the current code lacks: take the shifted
  path only when the resulting width does not exceed the caller's `bits`.
- **Soundness obligation.** As proposed this is a **restructuring, not a
  constraint removal** — the emitted predicate is the same bound, relocated onto
  an already-constrained witness. It is valid only when the shift is exact
  (`r == 0`). **Breaking counterexample:** omit the width guard and pass a
  non-constant `offset` with `rhs = 2^k`; the emitted check widens to `k+1` bits,
  which is *wider* than the general path and admits values the caller's budget did
  not. The `try_into_u128` gate currently shields this by accident; the extension
  removes the shield.
- **Why it might be null.** This is a witness-topology change **inside the exact
  function upstream PR #10159 regressed** — that regression came from two
  comparisons ceasing to share a witness. This change moves in the opposite
  (safer) direction, increasing sharing, but it is still the #10159 shape and
  must be adjudicated by backend gate count, never by opcode count. Barretenberg's
  cost for a 128-bit `RANGE` is unknown from this checkout. Deleting a witness
  also renumbers everything downstream, perturbing backend gate scheduling.
- **Firing shape.** `let t = x as u128;` for `x: Field`; any `u128` comparison.
- **Verdict.** `promising` — the strongest genuinely-new candidate found: a clean,
  fixture-confirmed asymmetry against the 64-bit path, a broad firing shape, and a
  restructuring rather than a removal.
- **Effort / risk.** Low-moderate implementation (~15 lines + a width guard +
  re-blessing two snapshots + extending `fuzz_bound_constraint_with_offset`,
  which draws `any::<u128>()` and so never covers this path). **High verification
  burden** because of #10159.

### P3-2 — signed `>>` is lowered through the generic signed-division expansion

- **Site.** `ssa/opt/remove_bit_shifts.rs:263-304` (`insert_shift_right`, the
  `Signed` arm), whose output is then re-expanded by `expand_signed_math`
  (`ssa/mod.rs:305` then `:310`).
- **Current behaviour.** The signed arm casts back to a *signed* type and issues a
  **signed `Div`** (`remove_bit_shifts.rs:280-282`), which `expand_signed_math`
  expands in full — re-deriving sign information `remove_bit_shifts` already
  computed at `:273`. Upstream's own snapshot (`remove_bit_shifts.rs:1011-1039`)
  shows the handed-on `div v8, i32 4`.
- **Static cost, read off the source (not a measurement).** Counting calls to
  `AcirContext::euclidean_division_var` — the primitive that emits the Brillig
  quotient hint plus quotient/remainder range checks
  (`acir_context/mod.rs:780,916,927,941`) — the signed-`>>` lowering is the most
  expensive single operation in the signed family: **5** for a constant shift
  amount, **7** for a dynamic one. Unsigned `>>` by a constant is **1**.
- **Proposed rewrite.** Lower signed `>>` as unsigned shift plus sign extension,
  the standard two's-complement identity: extract the sign bit once
  (`Div(a_u, 2^{n-1})`, the canonical form `expand_signed_math` already uses),
  take the *unsigned* quotient, and add a correction term that is a compile-time
  constant when the shift amount is. Static count: **2** (constant) / **3**
  (dynamic).
- **Soundness obligation.** Noir's signed `>>` is arithmetic (floor), not
  truncating — the existing derivation is documented in-code at
  `remove_bit_shifts.rs:238-251`. The rewrite must be verified equivalent **by
  the SSA interpreter over the full `i8`/`i16` domain**, not by hand-checking
  cases. Four specific obligations: the result bound must come from the
  constraints, not from the `Cast` (ACIR `Cast` is a no-op —
  `acir/mod.rs:504-507`); the `pow == 0` path must be guarded
  (`remove_bit_shifts.rs:327-334` can return zero); the predicate-off case must
  hold (`euclidean_division_var` returns `q = r = 0` when the predicate is off,
  `acir_context/mod.rs:794-796`); and the sign extraction is *more* constraining
  than the current `Lt` form on unreduced `unchecked_*` values
  (`ssa/ir/dfg.rs:586-618`) — a completeness, not soundness, hazard.
- **Why it might be null.** Signed integers are uncommon in ACIR Noir and signed
  *shifts* rarer still; if the corpus contains no `i32 >> k` the win is exactly
  zero. And a 32-bit range constraint is a backend lookup, so a 5→2 reduction in
  euclidean divisions is emphatically **not** a 60% gate reduction.
- **Firing shape.** `fn main(x: i32) -> pub i32 { x >> 2 }`; signed fixed-point
  and signed-average (`(a + b) >> 1`) code.
- **Verdict.** `promising`, gated on the corpus containing the shape at all —
  which is the first thing the harness must answer.
- **Effort / risk.** Medium-high: rewrites the correctness-critical arm of
  `remove_bit_shifts`, invalidates its snapshots, needs exhaustive interpreter
  differential tests.

### P3-3 — LICM's constraint-strength-reduction is unreachable in the production pipeline

- **Site.** `ssa/opt/loop_invariant/simplify.rs:104-176`
  (`simplify_not_equal_constraint`), reachable only from the
  `Instruction::ConstrainNotEqual` arm at `:231-241`.
- **Current behaviour.** For `constrain x != i` with `i` the induction variable
  and `x` loop-invariant, the pass replaces the *per-iteration* constraint with a
  **single** pre-header range check (`simplify.rs:151-157`). This is the one
  transformation in LICM that turns N *different* constraints into one — the
  shape post-unroll CSE structurally cannot recover, because the N unrolled
  copies have different constant operands.
- **Gap.** `Instruction::ConstrainNotEqual` cannot exist when LICM runs. It is
  constructed in exactly two places: `ssa/opt/make_constrain_not_equal.rs:108`
  and the test-only SSA parser. `make_constrain_not_equal` is ACIR-only and
  *requires* flattening (`make_constrain_not_equal.rs:27-29,113-124`), sitting at
  `ssa/mod.rs:373` — after flattening (`:315`) and after **both** unrolling passes
  (`:298`, `:367`), by which point there are no loops. The pass's own comment says
  as much (`simplify.rs:160-163`). At LICM time the construct is still in the
  pre-transform `eq` + `constrain` shape (`make_constrain_not_equal.rs:70-76`),
  which LICM does not match.
- **Proposed rewrite.** Add an arm to `simplify_induction_variable` matching
  `Constrain(v, false, _)` where `v` is defined by `Binary { operator: Eq, .. }`,
  and route it into the existing, already-tested `simplify_not_equal_constraint`
  unchanged, guarded by the identical preconditions and a single-use check on `v`.
- **Soundness obligation.** The rewrite replaces "for every visited `i`,
  `x != i`" with "`x < lower` or `max < x`" — an exact equivalence **only if** the
  visited set is exactly the contiguous interval. All four existing preconditions
  are load-bearing: the loop is entered (`does_execute`), there is no `break`
  (`no_break`), the block runs every iteration (`!is_control_dependent`), and the
  step is 0 or 1 (`simplify.rs:159-168`). **Breaking counterexample:** step 2 over
  `[0,4)` forbids only `{0,2}` but the rewrite forbids `{0,1,2,3}` — over-
  constrained, i.e. valid witnesses rejected. The dangerous *under*-constraining
  direction is a zero-trip loop, already blocked by `does_execute`.
- **Why it might be null.** The firing shape needs the raw induction variable as
  one side of the inequality. The common uniqueness idiom is
  `assert(arr[i] != target)`, where the varying operand is an array element —
  untouched by this. It is plausible the corpus contains zero instances, which is
  exactly what happened to the program's `sq-seust` candidate (0 firings across
  556 packages, program §10.8).
- **Firing shape.** `for i in 0..64 { assert(x != i); }`.
- **Verdict.** `promising` in kind (it is the only loop-family candidate not
  absorbed by post-unroll CSE), `likely-null` in frequency. **A firing-count
  instrumentation run over the corpus should precede any implementation** — the
  `sq-seust` protocol (env-gated per-site counter) applies directly and is cheap.
- **Effort / risk.** Small-medium; the transformation already exists and is
  tested. Failure mode is over-constraining (a completeness bug), so every
  precondition needs interpreter differential testing rather than argument.

### P3-4 — the value merger emits two identical reads per unchanged index

- **Site.** `ssa/ir/dfg/simplify/value_merger.rs:304-337`
  (`maybe_optimized_array_get`), consumed at `:212-223` and `:263-293`.
- **Current behaviour.** Each side of the merge independently materialises its own
  `ArrayGet` instruction; the insertion path simplifies but does not CSE, so the
  two ValueIds differ, the `then_value == else_value` short-circuit (`:99`,
  `:154`) misses, and a full conditional merge is emitted for an index **neither
  branch wrote**. Upstream's own committed snapshot shows it
  (`ssa/opt/remove_if_else.rs:669-676`): two byte-identical `array_get`s on the
  same array at the same constant index, followed by the long-form
  `c·v + (1-c)·v`.
- **Gap.** Acknowledged in-tree: `remove_if_else.rs:651-654` records that this
  optimisation existed, was removed by upstream PR #8142, and is *"Pending:
  investigate if this can be brought back"* under issue **#8145**. Post-unroll CSE
  removes the duplicate *read*, but no distributive peephole recovers
  `c·v + (1-c)·v → v`.
- **Proposed rewrite.** Memoize the resolved-array/index pair within a single
  merge; when both sides resolve to the same base array, emit one read and return
  it directly, skipping the numeric merge.
- **Soundness obligation.** The memo key must be the **post-resolution** base
  array, not the input value. **Breaking counterexample:** key on the raw
  `then_value`/`else_value` with `then = array_set(A, i, x)`, `else = A`; at index
  `i` the two sides are genuinely different and collapsing them drops the
  conditional write entirely — the circuit would accept `x` regardless of the
  condition. The memo must also not outlive the forced-predicate window the merge
  driver establishes (`remove_if_else.rs:213-253`).
- **Why it might be null.** The dominant `if c { a[i] = v }` shape never reaches
  the merger — `flatten_cfg::try_optimize_array_set_merge` intercepts it — and a
  merged array read only at constant indices is folded away by `MakeArray` folding
  plus DIE. The residue is confined to merged arrays that stay live (dynamic-index
  read, returned, or passed to a call). The in-tree test that exhibits the waste
  feeds hand-written SSA directly to `remove_if_else`, so **it does not prove the
  shape survives the real pipeline**.
- **Firing shape.** Both branches write different dynamic indices, then the array
  is read at a dynamic index.
- **Verdict.** `promising` with a caveat that dominates it: **PR #8142 removed
  this code for a reason that was not readable from this checkout.** Recovering
  that reason is a hard prerequisite — if it was removed for soundness or
  compile-time, reimplementing it is a regression, not an optimisation.
- **Effort / risk.** ~30 lines; risk **medium-high** for the reason above.

### P3-5 — `try_optimize_array_set_merge` bails when both branches write

- **Site.** `ssa/opt/flatten_cfg.rs:1219-1300`
  (`try_optimize_array_set_merge_inner`), specifically the chain walk starting at
  `then_value` (`:1245`) with base predicate `array == else_value` (`:1214`).
- **Current behaviour.** If `else_value` is itself an `array_set` result the walk
  fails and the generic whole-array `IfElse` is emitted (`:929-936`), costing
  O(N) in `remove_if_else` instead of O(chain length).
- **Proposed rewrite.** When both sides are `array_set` chains rooted at a common
  base, emit the merged sets for the then-chain under the condition and the
  else-chain under its negation, **threaded** (not parallel).
- **Soundness obligation.** Each merged set must be a no-op when its own predicate
  is false — which the existing `create_merged_array_set` provides by reading and
  writing back (`flatten_cfg.rs:1040-1063`). **Breaking counterexamples:** apply
  the two rewrites in parallel rather than threaded and `if c { a[i]=x } else
  { a[j]=y }` loses a write when `i == j` at runtime; forget to apply the existing
  `safe_index` machinery (`:1089-1128`) to *both* chains and an index in bounds
  only on its own branch faults under the forced predicate; and both chains must
  be pushed to `superseded_array_sets` or a live write is deleted.
- **Why it might be null.** The both-branches-write shape is much rarer than the
  single-branch one, and when both chains are long relative to N the rewrite can
  be *worse* than one N-way merge — it needs a chain-length-versus-N guard
  mirroring the existing cost reasoning at `:1278-1280`.
- **Verdict.** `promising`; **highest risk in the register** — this is the
  predication and aliasing core of flattening, where a wrong side-effects window
  is precisely an under-constraining bug.
- **Effort / risk.** Moderate effort, high risk.

### P3-6 — corroboration, not a new candidate: the program's own #13263 rule fires on compiler-generated code

The signed/wide-lowering examination independently re-derived, from a completely
different starting point, the rule that **is** the program's PR #13263: simplify
unsigned `Div` to zero when the dividend's bound proves it smaller than a
constant divisor (`ssa/ir/dfg/simplify/binary.rs:144-159`). It arrived there by
noticing that `check_u128_mul_overflow` (`ssa/mod.rs:374`) emits
`Div(x, 2^64)` guards whose dividend is frequently a value widened from a
narrower type, and that `checked_to_unchecked` — which computes the same bound —
does not run until `ssa/mod.rs:396`, 22 pass slots later.

This is **not a new opportunity**, and no bead should be spawned for it. It is
worth carrying into the paper for a different reason: it is independent evidence
that #13263's rule fires on **compiler-generated** `Div`s and not only on
hand-written ones, which bears directly on the honest reading of that PR's
focused-fixture number. It is a hypothesis, not a measurement: confirming it
needs a firing-count instrumentation run, which is a P1-harness task.

## 5. Null and already-handled results (the honest negative half)

These are first-class outputs. The paper's §5 needs them: a survey that reports
only its hits is not a survey.

### 5.1 LICM (`loop_invariant.rs`) — real pessimisms, absorbed for gate count

Four genuine pessimisms were found: the `ArrayGet` loop-bounds fast path
recognises only a *bare* induction variable as the index, so any derived index
(`i + 1`, `i * elem_size`, a cast) misses (`loop_invariant.rs:743-768`); the same
check compares against the semantic length rather than the semi-flattened length
`is_safe_index` uses (`ir/dfg.rs:838-849`) — a latent inconsistency that is
currently *unreachable*; narrowing `Cast` is classified as never-hoistable rather
than hoistable-under-predicate (`:886-892`, with a stated rationale at `:887`);
and loop blocks are visited in block-id order rather than reverse post-order, so
an invariant chain can be only partially hoisted, with no second LICM to finish
(`:419`, acknowledged at `:341-342`).

**All four leave behind loop-invariant instructions**, whose unrolled copies are
identical under an identical predicate and are collapsed by post-unroll CSE
(§2.2). For ACIR — the only runtime that produces gates — LICM's *hoisting* half
looks largely redundant with post-unroll CSE; its real payoff is pre-unroll SSA
size, compile time, and Brillig. **The upstream framing of #10439/#10438 ("LICM's
`ArrayGet` hoisting is too pessimistic") is therefore probably about SSA output,
not about proving cost** — a correction worth carrying upstream once measured.

### 5.2 Unrolling (`unrolling.rs`) — both named signals moot

- **Loop unswitching (#6631).** In a CPU the win is removing a branch from the
  loop body. After `flatten_cfg` there **are no branches** — both arms are emitted
  and merged (`flatten_cfg.rs:29-36`), so the branch costs one merge, not a
  branch. Unswitching duplicates the whole loop, so any shared body statement is
  computed under **both** predicates; the transform therefore trades ~N merges for
  up to N×|shared body| duplicated arithmetic. **The risk direction is a gate
  regression, not a win.** It also hands the unroller two loops instead of one, a
  direct hit on the known compile-time sensitivity.
- **Induction-variable strength reduction / elimination (#6629).** Moot in ACIR:
  the unroller refuses to proceed unless the induction variable's initial value is
  a constant (`unrolling.rs:1227-1234`) and then substitutes concrete values per
  iteration (`:1305-1311`) through an inserter that simplifies on insert — so
  `i * k` is a literal *during unrolling*, before any later pass sees it. In
  Brillig, the compiler's own cost model prices unchecked `Add` and unchecked
  `Mul` identically (`brillig/.../target_cost.rs:77-92`), so a mul→add reduction
  is a 1→1 no-op; the one large delta (checked→unchecked) is **already
  implemented** at `loop_invariant/simplify.rs:306-349`.

### 5.3 Array/memory merging — the #5501 ask is already implemented

`try_merge_only_changed_indices` does not exist at this pin. The "merge only the
changed indices" optimisation is implemented in `flatten_cfg`, which states the
intent verbatim (`flatten_cfg.rs:1187-1200`) and walks an `array_set` chain up to
a depth bound (`:1201-1300`, `MAX_ARRAY_SET_CHAIN_DEPTH` at `:168`);
`array_set_window_optimization.rs` is a second, independent feeder for the same
goal (`:15-17`). The named function was removed upstream by **PR #8142** with the
follow-up tracked as **#8145** (`remove_if_else.rs:651-654`).

**Correction to the prior note's cost claim.** "~2N constraints per merged
N-array for one read" is wrong in both directions at this pin. Two short-circuits
already elide value-identical indices (`value_merger.rs:99-101`, `:154-156`); and
for a single *constant*-index read the merged `MakeArray` is folded and dead-code
eliminated, collapsing the cost to O(1). The ~2N regime is real but conditional:
it needs the merged array to stay live — a dynamic-index read, or the array being
returned, nested, or passed to a call.

`mutable_array_set.rs` was examined and yields no candidate: it is a last-use
liveness analysis that sets a reuse flag, not a merging pass, and its
conservatism is regression-tested against a specific upstream bug (`:271-289`).

### 5.4 Memory SSA (`mem2reg.rs`) — not proof-cost-relevant

Two independent findings, either sufficient. **First, reference memory operations
never reach ACIR generation**: `Allocate` returns `RuntimeError::UnknownReference`
and `Store`/`Load` are `unreachable!()` (`acir/mod.rs:531-541`), enforced by a
pipeline-boundary validator (`ssa/validation/mod.rs:1345-1371`, wired at
`ssa/mod.rs:417-423`). mem2reg's ACIR job is **binary (does it compile), not
quantitative** — there is no marginal cost gradient to optimise along. The
gate-bearing memory surface is `ArrayGet`/`ArraySet` lowering to memory
operations (`acir/arrays.rs:762-793,882-916,1312-1376`), a different pass family.

**Second, the hypothesised gap is already closed.** mem2reg runs on all functions
immediately after flattening (`ssa/mod.rs:325`) with a comment stating the
cross-block motivation and noting it is *trivial for ACIR because ACIR is
single-block post-flatten* (`:321-324`); and mem2reg is not per-block — it is
full Cytron iterated-dominance-frontier SSA construction (`mem2reg.rs:1-14`,
`:181-215`). Every remaining precision gap found (function-wide disqualification
on any escaping use, `mem2reg.rs:498-527`) is **Brillig-only**, which the
program's own rule excludes.

The one plausible bridge from this area to gates — the O(N) array merge at a
predicated store — is **already implemented on both the store path and the
block-parameter path** (`flatten_cfg.rs:1130-1300`).

### 5.5 Signed / wide lowering — a cost table, one candidate, no other gaps

The durable deliverable here is a static count, read off the source, of how many
`euclidean_division_var` calls each signed/wide operation emits — the primitive
that dominates this family. Highest first: **signed `>>`** (5 constant / 7
dynamic — candidate P3-2); **signed checked `+`/`-`/`*`** (4 each); **signed
`<`**, **signed `/`**, **signed `%`**, and **u128 `/` or `%` by a dynamic
divisor** (3 each); **unsigned `>>` dynamic**, **unsigned `<<` dynamic**, and
**u128 checked `*` with two non-constant operands** (2 each); **unsigned `>>` by
a constant**, **unsigned `<` / `>`**, **`Truncate`**, **unsigned `%`** (1 each);
**`Eq`**, **`Xor`/`And` on `u1`**, and **`<<` on the happy path** (0). These are
**static counts, not measurements**, and an ACIR euclidean division does not map
to a fixed number of backend gates.

Two things were checked and found *not* to be gaps. Every expansion pass is
followed by at least one constant-folding pass, and the simplifier runs at
insertion time inside the expansion passes themselves — so there is **no
"expanded and never cleaned" gap**. And two constructs that look wasteful are
load-bearing with in-code derivations: the signed `<` lowering (a 12-line proof of
its identity is inline at `expand_signed_math.rs:116-126`) and the MIN-divided-by-
minus-one overflow constraint (`:147-169`), without which `i8::MIN / -1` silently
wraps instead of failing as Rust does.

One further redundancy was found and judged **likely-null**: the same
"extract the sign bit" primitive is spelled three incompatible ways across three
passes (`expand_signed_math.rs:104-108`, `expand_signed_checks.rs:180-188`,
`remove_bit_shifts.rs:271-273`), defeating structural CSE. The saving is one
euclidean division per value per function where the idioms co-occur, the safe
version is a three-file refactor, and the two spellings are *not* interchangeable
on unreduced values — swapping one for the other adds a constraint.

### 5.6 Conditionals (`basic_conditional.rs`) — Brillig-only

`flatten_basic_conditionals` early-returns on every non-Brillig function
(`basic_conditional.rs:280-284`), the module doc says "unconstrained" (`:1-2`),
the pass label says so (`ssa/mod.rs:352`), and the cost model it optimises is an
execution-opcode metric (`:88-121`), not a constraint metric. Brillig emits no
gates. **Area closed; no candidates.**

### 5.7 ACIR-gen comparison lowering — the prior null result re-confirmed at this pin

All three load-bearing findings of `sq-jfkwk` (program §10.7) were re-checked
against `8f33502e` and **all three are confirmed**:

- For a constant `rhs`, `euclidean_division_var` tightens the quotient bound to
  exactly 1 bit for `more_than_eq_var` (`acir_context/mod.rs:874-889` with
  `:1216-1248`), corroborated by upstream's committed `lt_u8` fixture
  (`acir/tests/instructions/binary.rs:490-533`) which contains a literal 1-bit
  range constraint. The quotient check *already is* the boolean constraint the
  rejected sketch would have emitted.
- `less_than_var` returns an expression, never a witness
  (`acir_context/mod.rs:1254-1266`), and the frontend lowers `>=` as
  `Not(Lt(..))` (`ssa/ssa_gen/context.rs:1022-1025,1051`), so both directions
  share one hint and one pair of range checks with the negation free.
- The remainder range check is intentional, with the reason stated in-code
  (`acir_context/mod.rs:918-926`); both branches are visible in committed
  fixtures, and the ACVM `RangeOptimizer` deduplicates the power-of-two case
  (`redundant_range.rs:153-157,262-266`).

One honest caveat the prior record did not carry: the 1-bit check is an ACIR
`RANGE` black-box call, not a boolean `AssertZero` gate. Whether Barretenberg
lowers a 1-bit `RANGE` to exactly one boolean gate is a **backend** fact, and
Barretenberg is not in this checkout — so the conclusion holds at the ACIR level
and is asserted, not verified, at the backend level.

### 5.8 ACVM optimizers — null, and the source of the §2.3 discount

The `general.rs` on-the-fly term-merge TODO (#10109) is **largely already done**:
`Expression::add_mul` is a sorted merge-join that sums coefficients for equal
witnesses and drops zeros as it goes
(`acvm-repo/acir/src/native_types/expression/mod.rs:205-311`), and every
expression `Add`/`Sub`/`Mul` routes through it. The residue `GeneralOptimizer`
cleans is narrow (chiefly zero-coefficient debris from scalar multiplication) and
the TODO is in any case a **compile-time** concern with no circuit-output change
— **zero gate impact by construction**.

The genuinely open ACVM gap is cross-opcode partial fan-in sharing, flagged
in-tree as #10192 (`common_subexpression/csat.rs:240-242`): the CSE is exact-chunk
only, so two opcodes sharing a *subset* the greedy slicer chunks differently share
nothing. It is classified **gates-risk-inverted** and should not be pursued:
hoisting a shared subset adds an opcode and a witness and only pays at three or
more uses; at exactly two uses `MergeExpressionsOptimizer` immediately undoes it
(`merge_expressions.rs:102`), burning one of the three available passes. And a
wide linear combination is split into several bounded-fan-in backend gates either
way, so the shared and un-shared forms can arithmetise identically.

Three smaller items were classified and are recorded so they are not re-derived: a
merge-produced trivially-satisfied constraint can survive to the backend because
the trivial-constraint filter runs only once, before the loop
(`optimizers/mod.rs:64-81` versus `merge_expressions.rs:138-139`) — correct and
cheap to fix but likely never fires and plausibly gate-neutral; merging harder by
removing the one-merge-per-opcode-per-pass break (`merge_expressions.rs:150-152`)
is **gates-risk-inverted** and additionally re-opens the shape of a documented
past under-constraining bug (`:398-415`); and the refusal to merge a witness
occurring in any multiplication term (`:254-263`) is **structurally necessary** —
substituting would raise the degree above two, which the representation cannot
express.

## 6. Draft paper section — "New opportunities surfaced while writing"

*(Intended for §5 of `research/noir-optimization-paper.md`'s outline. Written to
be dropped into the Typst source once P1 exists to source its figures. Every
figure position is left as an accessor, not a literal, per the paper-factory
honesty rule.)*

> **5. New opportunities surfaced while writing**
>
> The mandate for this section was to carry a measured candidate through each
> compiler stage the program had not yet examined at implementation level. We
> report both what we found and, at greater length, what we found *not* to be
> there — because the negative results have a common structural cause that is
> the more transferable contribution.
>
> **5.1 Method.** We examined, at source level and against a single pinned
> commit, every stage outside the value-range / bit-width / simplify
> neighbourhood in which the program's seven existing pull requests sit: loop
> optimisation (invariant code motion, unrolling), array and memory merging
> around control-flow flattening, memory-SSA promotion, conditional simplify,
> signed and wide-integer lowering, ACIR generation, and the ACVM-level circuit
> optimisers that run last before the backend. Each stage was examined under a
> standing instruction to seek a null verdict first and to treat a documented
> in-code rationale as evidence of deliberate design rather than as an
> opportunity.
>
> **5.2 Why the classical catalogue is mostly null here.** Three absorption
> layers separate an SSA-level pessimism from the proving cost, and each
> independently voids a family of textbook optimisations.
>
> First, loops in constrained functions are *always* fully unrolled or the
> compile fails; the frontend forbids the unbounded loop forms outright.
> Unrolling substitutes the induction variable with a literal through an
> inserter that simplifies at insertion time. Loop unswitching and
> induction-variable strength reduction — both requested upstream — therefore
> have no subject: by the time any later pass runs, a loop body is straight-line
> code with literal indices. Worse for unswitching, flattening emits both arms of
> a conditional unconditionally, so a branch does not cost a branch; duplicating
> the loop to remove it computes the shared body under both predicates, and the
> risk direction is a *regression*.
>
> Second, dominance-aware common-subexpression elimination runs repeatedly after
> unrolling. After unrolling, N copies of a loop-invariant instruction are the
> same instruction under the same predicate — so a hoisting miss in
> loop-invariant code motion is, for proving cost, very likely a no-op. We found
> four genuine hoisting pessimisms and judged all four absorbed. This suggests
> the open upstream issues framing invariant-code-motion hoisting as too
> pessimistic are describing intermediate-representation size, not proving cost.
>
> Third, the ACVM performs circuit-global, scale-invariant common-subexpression
> elimination and refunds any intermediate witness used in exactly two
> constraints, by Gaussian elimination. This discounts the entire
> "avoid materialising this temporary" and "share this repeated subexpression"
> family — and means an SSA pass that *creates* a two-use temporary has it
> silently undone downstream.
>
> A fourth observation compounds these. The ACVM optimiser's own convergence
> criterion is ACIR opcode count: its transform loop terminates when the opcode
> count stops changing, capped at three passes, with a checked-in circuit that
> provably exits on the cap rather than on convergence. The layer immediately
> before the backend is tuned on the metric this paper's methodology argues is
> not proving cost. Several otherwise-attractive candidates at that layer are
> for this reason *gates-risk-inverted*: they reduce opcodes by widening
> expressions, and a wide linear combination is split into several
> bounded-fan-in backend gates regardless.
>
> **5.3 Where candidates survive.** The surviving candidates concentrate exactly
> where none of the three absorption layers reaches: expensive *lowering
> formulations*, where a cheaper equivalent of the same predicate exists; and
> N-distinct-constraints-to-one rewrites, which common-subexpression elimination
> structurally cannot perform because the N copies differ. We carried
> `headline(candidate_count)` such candidates out of this pass. The strongest
> is an asymmetry in the bound-constraint helper, whose constant fast path is
> gated on a 128-bit conversion and therefore silently declines for exactly the
> 128-bit operations — every `u128` comparison and every field-to-`u128`
> truncation — which consequently pay a redundant full-width range check on a
> fresh witness that the 8-bit and 64-bit paths do not pay. Upstream's own
> committed expected-output fixtures exhibit both sides of the asymmetry. The
> proposed change is a restructuring rather than a constraint removal, which
> matters: it relocates a bound onto an already-constrained witness rather than
> dropping one.
>
> **5.4 Honest status of this section.** Every candidate here is an *unmeasured
> hypothesis*. (Measurement status, to be sourced through the evidence accessor
> rather than typed as prose: at the time of writing, no backend gate measurement
> of any candidate in this section had been performed, because the proving backend
> was not available in the measurement environment.) We state this rather than presenting
> code-level reasoning as though it were evidence — which is the same discipline
> that produced this program's earlier negative results, where a candidate that
> looked compelling at the ACIR level turned out to be opcode-for-opcode
> identical to the existing lowering, and another fired zero times across a
> 556-package corpus. A candidate becomes a result in this program only when a
> backend gate win reproduces; until then it is a specification.

## 7. Beads — the spawn gate, and why nothing was spawned

The brief's spawn rule is: *for each candidate that reproduces a `bb`-gates win
via the P1 harness, spawn a measured-PR child bead under `sq-uuvac`*.

**Zero beads are spawned, because the gate is unmet by construction.** No
candidate reproduced a gate win because **no candidate could be measured**: the
P1 harness (`sq-i50o4`) has not landed, this box has no Noir toolchain, and the
program's own records show `bb` was absent even on the Noir work box in the two
prior runs that successfully built the compiler (program §10.8, §10.9). Spawning
beads anyway would present code reading as though it were measurement — exactly
the failure mode this program's honesty rules exist to prevent.

The candidates below are therefore recorded as **bead specifications, ready for
`bd create` under `sq-uuvac`, each blocked on `sq-i50o4`**. They are specs, not
beads; this record creates nothing. Note also that the executing environment
could not create beads in any case (`bd` is not installed here, and this bead's
orchestration contract forbids mutating the bead store).

| Spec | Target (noir) | Tier | Blocked on | Exit condition |
|---|---|---|---|---|
| **P3-1** bound-constraint 128-bit asymmetry | `acir/acir_context/mod.rs` | opus | `sq-i50o4` | Backend gate win reproduces on a `u128`-heavy circuit **and** on the two affected committed fixtures; park with a findings note otherwise. Witness-topology change in the #10159 function — opcode counts are not admissible evidence. |
| **P3-2** signed `>>` reformulation | `ssa/opt/remove_bit_shifts.rs` | opus | `sq-i50o4` | **Step 0: does the corpus contain a signed shift at all?** If not, park before writing code. Then interpreter-differential equivalence over the full `i8`/`i16` domain, then a gate win. |
| **P3-3** LICM constraint strength reduction | `ssa/opt/loop_invariant/simplify.rs` | opus | `sq-i50o4` | **Step 0: firing-count instrumentation over the corpus, per the `sq-seust` protocol.** Zero firings ⇒ park, no PR. Failure mode is over-constraining, so every precondition needs interpreter differential tests. |
| **P3-4** value-merger duplicate reads (#8145) | `ssa/ir/dfg/simplify/value_merger.rs` | opus | `sq-i50o4` | **Step 0: read upstream PR #8142 and establish why this code was removed.** If removed for soundness or compile time, close the spec. Then demonstrate the shape survives the real pipeline (the in-tree test does not). |
| **P3-5** both-branches-write array merge | `ssa/opt/flatten_cfg.rs` | opus | `sq-i50o4` | Chain-length-versus-N guard; `safe_index` applied to both chains; both chains pushed to the superseded set; then a gate win. Highest under-constraining risk in the register. |

Every spec inherits the program's common acceptance protocol (program §10.2) and
the upstream-contribution protocol (program §6): draft PR, explicit
generative-AI disclosure, @jeswr's author review before any maintainer review,
one optimisation per PR, and no agent arms anything.

**P3-6 deliberately has no spec** — it is the program's own #13263, and the
finding is corroborating evidence for that PR's reading, not new work.

The genuine bottleneck this record exposes is that **`sq-i50o4` (P1) is not
merely the paper's reproducibility prerequisite — it is the gate on this entire
candidate register, and on four other beads already open and blocked on the same
missing capability** (`sq-eesz3`, `sq-felqr`, `sq-jfkwk`, and the gate half of
`sq-jthy1`). The single highest-value next action in the whole program is
acquiring a `bb` binary in the measurement environment and landing P1 around it.

## 8. Corrections to propagate

1. **`sq-m3l62` must be re-spec'd before a fleet agent picks it up.** Program
   §10.3 targets `ssa/opt/flatten_cfg/value_merger.rs` and the function
   `try_merge_only_changed_indices`. The file moved to
   `ssa/ir/dfg/simplify/value_merger.rs` and the function no longer exists —
   removed upstream by PR #8142. The live tracker is **#8145**, not #5501, and
   the original ask is already implemented in `flatten_cfg`. The residual
   opportunity is **P3-4**, which is a different and narrower thing.
2. **The paper's §2.2 gap table needs four rows corrected** — the value-merger
   row (as above), the mem2reg row (not proof-cost-relevant; the hypothesised
   cross-block-after-flattening gap is already closed), the `basic_conditional.rs`
   row (Brillig-only), and the two loop rows (both named signals moot under full
   unrolling). These are applied in this change.
3. **"~2N constraints per merged N-array for one read"** should carry the
   qualifier from §5.3: it holds for dynamic-index or escaping merged arrays, not
   for a constant-index read.
4. **Upstream issue numbers in the program's records are unverified and at least
   one is stale.** #5501 is superseded by #8145. No issue thread was read from
   this bead. Any upstream comment drafted from these records must be re-checked
   against its live thread first — the standing rule from program §10.5.

## 9. Status & honesty

- **DOC-ONLY design-for-review.** No compiler source is modified, no upstream PR
  is opened, no bead is created, no GitHub API call was made.
- **Nothing was measured.** No build, no `nargo`, no `bb`, no benchmark, no
  firing counts. Every candidate is an unmeasured hypothesis and every
  euclidean-division count is a static count read off the source. No number in
  this record is a performance measurement, and none should be quoted as one.
- **The pin is `8f33502e` and drift could not be diffed** (shallow clone). Prior
  program records analysed older pins; where this record says something moved or
  was removed, that is a description of the current tree corroborated by an
  in-tree comment, not a `git log` finding.
- **Backend behaviour is inferred, not verified.** Barretenberg is not in the
  checkout. Every statement about what an ACIR construct costs in gates is
  inference from the emitted ACIR.
- **Upstream issue status is unverified.** Issue numbers are carried from prior
  program records; at least one (#5501) is demonstrably superseded.
- **None of the program's seven PRs is merged**, verified at this pin for the
  four fresh drafts (§1). The paper must never present any as landed.
- **No *cryptographic* ZK claim — but not "no soundness concern".** These are
  circuit-size (proving-cost) optimisations: they make and require no
  zero-knowledge privacy or proof-system soundness property of the backend. They
  are nonetheless **constraint-soundness-sensitive**, because a wrong rewrite
  under-constrains the circuit. The per-candidate equivalence obligations in §4
  and the fail-closed spawn conditions in §7 (interpreter differentials,
  precondition audits, the P3-4 "why was this removed upstream" prerequisite)
  are the mitigation, and **none of them has been discharged here**.
