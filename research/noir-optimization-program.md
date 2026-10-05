# Noir upstream optimization program — whole-compiler assessment (sq-rbfga)

<!-- [OPUS-4.8] sq-5reoy (#1599): the in-tree `zk/ieee754` and `zk/xpath` Noir trees were externalized to the `sparq-org/noir_IEEE754` (v0.10.0) and `sparq-org/noir_XPath` (v0.2.0) face repos and REMOVED from this repo; `zk/compose` now consumes the released `sparq_ieee754` as a pinned Nargo git dependency. Any `zk/xpath/…` / `zk/ieee754/…` path below is a HISTORICAL in-tree reference — the live source is the corresponding face repo. -->

> 🤖 **SPARQ agent** — assessment produced for epic `sq-uuvac` (Noir upstream
> optimization program), bead `sq-rbfga`. [FABLE-5]
> Analysis of noir-lang/noir @ `0df14918` (2026-07-04, version `1.0.0-beta.22`-dev),
> workspace `~/noir-optim-workspace/noir` (outside the sparq repo, per the epic).
> No upstream PRs were opened from this assessment.

## 1. Scope and doctrine

The maintainer (@jeswr) previously landed three upstream **draft** optimization PRs
with a weaker model's help. This program re-derives similar optimizations **fresh**
(no code reuse) plus any other optimizations found anywhere in the compiler, as a
series of minimal, individually-measured upstream PRs. This record is the plan;
implementation beads are children of `sq-uuvac`.

**Program-level constraint discovered during the survey (load-bearing):**
noir's `CONTRIBUTING.md` explicitly states that PRs consisting of *entirely
AI-generated code* are likely to be closed. jeswr's established mitigation — and
this program's mandatory protocol — is: **draft PR first, explicit disclosure
("developed with support from generative AI; author review before maintainer
review"), the author (@jeswr) reviews and takes ownership before requesting
maintainer review**, every claim measured, every PR issue-linked. Transparent,
engaged, measured contributions are the only acceptable mode. See §6.

## 2. Digest of jeswr's three upstream PRs (themes only — code not reused)

All three are OPEN drafts (checked 2026-07-04). **No human maintainer review has
been posted on any of them yet** — only a Copilot review on #12927 — so the
binding "reviewer objections" so far are the constraints the PRs impose on
themselves (rejected-experiment notes), plus repo automation rules.

| PR | Theme | Key files touched | Measurement style |
|---|---|---|---|
| [#12780](https://github.com/noir-lang/noir/pull/12780) | Track unsigned value ranges from unconditional `range_check` / `constrain lhs == rhs`; propagate through lossless casts, `not`, selected unsigned arithmetic; feed `checked_to_unchecked` | `ssa/ir/dfg/range_analysis.rs` (new), `ssa/ir/dfg.rs`, `ssa/opt/checked_to_unchecked.rs`, `acir/mod.rs` | Focused `test_programs/benchmarks/live_bit_width_*` fixtures; `cargo test -p noirc_evaluator` |
| [#12781](https://github.com/noir-lang/noir/pull/12781) | Simplify unsigned `lhs / C → 0` and `lhs % C → lhs` when `max(lhs) < C`, `C != 0` (stacked on #12780) | `ssa/ir/dfg/simplify/binary.rs` + the range stack | Local `bb gates -s ultra_honk`: `live_bit_width_large_constant_div_mod` **2953 → 88** gates; `live_bit_width_ranges` 3753 → 3720 |
| [#12927](https://github.com/noir-lang/noir/pull/12927) | Full SSA value-range analysis (unsigned + signed two's-complement domains): drop redundant range checks, narrow `Lt` widths, checked→unchecked; plus compile-time work (one fixed point per function) | same area + `ssa/opt/range_check_elision.rs` (new pass) | ACIR opcodes + bytecode per benchmark (`live_bit_width_ranges` 91→63 opcodes, −31%); compile-time table; standard circuits verified unchanged |

**Design constraints recorded inside those PRs (treat as constraints for our fresh PRs):**

- Constraint-derived ranges are only sound as *global* facts in single-block
  functions without side-effect predicates — branch-local/predicated constraints
  must not be treated globally (#12780 review notes; #12927 ⭐ commit `08806ed`).
- Backward propagation from `Eq`/`Lt` *results* was tried and **rejected**: it
  risks circularly narrowing the very comparison that enforces the assertion, and
  did not improve UltraHonk gates (#12780 review notes).
- Broader backward bit/range inference experiments were gate-neutral or
  **regressed** UltraHonk gates and were dropped (#12781 review notes).
- Every inferred range must be a **sound over-approximation** — a too-tight range
  drops a needed constraint and under-constrains the circuit (#12927 module-doc
  contract; four soundness fixes shipped with regression tests, incl. unchecked
  add/mul exceeding its type width through a widening cast).
- Compile-time discipline: an O(instructions²) inline recomputation did not finish
  `sha512_100_bytes` in 12 min; the fix is one fixed point per function, elision as
  a dedicated pass, not inline simplification (#12927 `bc74335`).
- Repo automation: do **not force-push** after team review starts (sticky bot
  comment on all three PRs).

## 3. Compilation pipeline map (HEAD `0df14918`)

### 3.1 Frontend

`noirc_frontend`: lexing/parsing → **elaborator** (name resolution, type checking,
comptime interpreter) → **monomorphization** (`noirc_frontend/src/monomorphization/`;
identical instantiations are deduped via a multi-level cache keyed on
(function id, unconstrained-ness, type, turbofish, bindings) —
`monomorphization/mod.rs:425-437`) → **SSA generation**
(`noirc_evaluator/src/ssa/ssa_gen/`). Array-index bounds checks are emitted here:
power-of-two array lengths get `RangeCheck`, non-power-of-two get `Lt`+constrain
(`ssa_gen/mod.rs:627-661`), with a known-odd cast-to-Field workaround
(`ssa_gen/mod.rs:643-645`, upstream issue #9191).

### 3.2 SSA pass pipeline

`noirc_evaluator/src/ssa/mod.rs:205-424` defines the primary pass list — ~85
sequential pass invocations (many repeated). Condensed phase map (each pass lives
in `ssa/opt/<name>.rs`):

1. **Early/infra**: black_box bypass, ArraySet/ArrayGet opts, expand signed
   checks, remove unreachable functions, Brillig-only Mem2Reg + load-store
   forwarding (LSF) + cleanups.
2. **Pre-inlining**: defunctionalization, lower refs at ACIR/Brillig boundary,
   inline simple functions.
3. **Inlining + memory**: Mem2Reg, LSF, array opts, purity analysis, full
   inlining, post-inline Mem2Reg/LSF, DIE, simplify, specialization.
4. **Loops**: LICM, constant folding, simplify CFG, unrolling (iterated).
5. **Lowering to branchless form**: `remove_bit_shifts` (dynamic shifts →
   bit-decomposition), array opts, `expand_signed_checks` (after bit-shift
   removal, which introduces signed divisions — ordering comment
   `mod.rs:308-310`), **`flatten_cfg`** (non-optional: ACIR has no control flow;
   both `if` arms cost), array-set window opt, Mem2Reg (cross-block, after
   flattening — `mod.rs:321-324`), `inline_functions_with_no_predicates`.
6. **Late cleanup (×4 DIE cycles)**: LSF, `remove_if_else`, constant folding,
   simplify CFG, re-unrolling, static asserts, `make_constrain_not_equal`,
   u128-overflow checks, `remove_truncate_after_range_check`,
   **`checked_to_unchecked`** (local per-value max-bits map — *no global range
   analysis exists on HEAD*; `ssa/opt/checked_to_unchecked.rs`), constraint-aware
   constant folding (×2), remove `EnableSideEffectsIf`, final DIE, mutable
   array-set opt.

Ordering constraints are documented in-line (e.g. LSF must precede DIE because DIE
removes Stores, `mod.rs:384-386`; remove-unreachable-instructions must precede
Mem2Reg, `mod.rs:378-381`). Instruction-level algebraic simplification lives in
`ssa/ir/dfg/simplify/` (`binary.rs`, `call.rs`, …) and is invoked on insertion by
many passes.

### 3.3 ACIR generation + ACVM transforms

`noirc_evaluator/src/acir/`: SSA → ACIR opcodes. Load-bearing emission sites:

- **Euclidean division** (`acir/acir_context/mod.rs:~850-940`): Brillig quotient
  hint + quotient range check + remainder range check + `r < rhs` bound
  constraint + the `a·p == (b·q + r)·p` relation. For constant `rhs` the bit
  bounds are tightened (`bit_size - rhs_bits + 1` for `q`); for dynamic `rhs`
  they default to full `bit_size`. The remainder range check is **deliberately
  always emitted** — the in-code comment notes it is redundant for power-of-two
  `rhs` and relies on the ACVM optimizer to drop it, but is *required* for
  non-power-of-two `rhs` (the bound constraint introduces a fresh witness).
- **Truncation** (`truncate_var`, `mod.rs:1145-1156`) is implemented as full
  euclidean division by `2^rhs`.
- **Comparisons** (`more_than_eq_var`, `mod.rs:1163-1228`): sign-bit extraction of
  `diff + 2^max_bits` via euclidean division by `2^max_bits`, at the full type
  bit width.
- **Range checks** (`range_constrain_var`, `mod.rs:1106-1135`): constants are
  checked at compile time; under a predicate the check applies to
  `predicate · variable`.
- **Bitwise AND/XOR** → black-box calls; range info is *not* re-applied to
  outputs at ACIR-gen (`acir/acir_context/black_box.rs:91`, documented, issue
  #1439); the ACVM `redundant_range` optimizer instead *assumes* the
  Barretenberg AND/XOR gadget constrains inputs+outputs to `num_bits`
  (`acvm-repo/acvm/src/compiler/optimizers/redundant_range.rs:172-181`). This is
  a documented backend contract, not a bug — but it is a **portability
  assumption** worth knowing when reasoning about range-check elision.

ACVM-level pipeline (`acvm-repo/acvm/src/compiler/optimizers/mod.rs:55-111`):
`general` (merge mul/linear terms — with an upstream TODO #10109 to do it on the
fly, `optimizers/general.rs:16`) → unused-memory elimination → `redundant_range`
(keeps the tightest range check per witness) → CSAT expression-width transformer.

### 3.4 Brillig generation

`noirc_evaluator/src/brillig/`: unconstrained code → Brillig VM bytecode. Brillig
opcodes are free at proof time; only unconstrained execution speed and bytecode
size are affected. Upstream appetite for Brillig micro-opts is **low** (closed
NOT_PLANNED: #5791, #2936; PR #11580 rejected on maintenance-cost grounds) —
deprioritized in this program.

### 3.5 Stdlib

`noir_stdlib/src/`: library-level circuit patterns. Cost-relevant hotspots:
`field/mod.nr` decomposition + modulus-check loops (`to_le_bits`/`to_le_bytes`
etc., lines ~36-153) and `lt_fallback` (32-byte scan, lines ~361-385); `pow_32`
(32-iteration square-and-multiply, lines ~181-190); `array/quicksort.nr`
(unconstrained — proof-cost-irrelevant). Upstream issue #6313 asks for tests that
near-inverse stdlib pairs (`to_be_bytes`/`from_be_bytes`) optimize away.

## 4. Upstream landscape

### 4.1 Known-wanted optimizations (open issues; higher value because pre-endorsed)

| Issue | Ask | Maintainer signal | Difficulty |
|---|---|---|---|
| [#8628](https://github.com/noir-lang/noir/issues/8628) | Don't emit `truncate` when the value is masked by `and` with a small constant | jfecher gave the implementation path: extend the max-bit tracking in `ssa/opt/remove_truncate_after_range_check.rs` to `and` | Low |
| [#9463](https://github.com/noir-lang/noir/issues/9463) | Remove redundant `LessThan`/range constraints (`assert(a<16); assert(a<17)`; `t[a%3]` OOB check) | Wants a `remove_truncate_after_range_check`-style pass; cross-linked to #9429 | Low-medium |
| [#9429](https://github.com/noir-lang/noir/issues/9429) | Track bool-condition facts / value ranges in the SSA optimizer | Concrete SSA examples in body; subsumes #9463 | Medium, high leverage |
| [#5501](https://github.com/noir-lang/noir/issues/5501) | Push `ArrayGet` backwards through `IfElse` to avoid full array merges (~2000 reads to merge two 1000-elem arrays for one read) | jfecher pointed at `try_merge_only_changed_indices` in `ssa/opt/flatten_cfg/value_merger.rs`; precedent PR #11512 merged | Medium |
| [#7161](https://github.com/noir-lang/noir/issues/7161) | Remove overflow checks on intermediate arithmetic dominated by a later check | Body notes a Brillig-side check may be needed | Medium |
| [#8055](https://github.com/noir-lang/noir/issues/8055)/[#8317](https://github.com/noir-lang/noir/issues/8317) | Fold chained arithmetic (`sub 0,x; add …`), collect constants | Prior art PR #8064 (TomAFrench) closed unmerged — read before re-attempting | Medium |
| [#6631](https://github.com/noir-lang/noir/issues/6631) | Loop unswitching | Accepted bytecode-size trade | Medium |
| [#6629](https://github.com/noir-lang/noir/issues/6629) | Induction-variable elimination / strength reduction | Exact rewrite specified | Medium-high |
| [#6624](https://github.com/noir-lang/noir/issues/6624) | Investigate `check_for_negated_jmpif_condition` for ACIR | Pure investigation | Low |
| [#10439](https://github.com/noir-lang/noir/issues/10439)/[#10438](https://github.com/noir-lang/noir/issues/10438) | LICM ArrayGet hoisting too pessimistic | Incremental | Low-medium |
| [#5027](https://github.com/noir-lang/noir/issues/5027) | Conditional RAM writes scale nonlinearly (1.65M vs 12k constraints) | TomAFrench isolated a removable `BoundedVec` capacity assert | High (sub-task easy) |
| [#4629](https://github.com/noir-lang/noir/issues/4629) | ACIR size grows quadratically with loop iterations (SSA is linear ⇒ acir-gen expression growth) | `as_witness` added as workaround; #6539 dup | High, high value |
| [#13046](https://github.com/noir-lang/noir/issues/13046) | Memory blowup during ACIR synthesis (Poseidon sponge) | aakoshh could not reproduce 48GB (saw 13GB); per-pass profile in comments | Medium-high |
| [#6313](https://github.com/noir-lang/noir/issues/6313) | Verify near-inverse stdlib pairs optimize away | Investigation + tests | Low |
| [#4972](https://github.com/noir-lang/noir/issues/4972) | Alternative ACIR-gen for predicated `array_get` (index-offset vs quadratic expression) | Measurement experiment suggested by TomAFrench | Low-medium |

Also in-code TODOs that are genuine opportunities: simplify `to_bits`/`to_radix`
to a range constraint when `limb_count == 1`
(`ssa/ir/dfg/simplify/call.rs:59,79`); `IfElse` merging blocked on NOT
instructions with distinct ValueIds (`ssa/ir/dfg/simplify.rs:343-344,361-362` —
implemented and **measured to never fire**; PARKED by `sq-seust`, §10.8);
on-the-fly expression term merging (`acvm/src/compiler/optimizers/general.rs:16`,
issue #10109).

### 4.2 Rejected / landmine list

- **PR #11580** (Brillig self-move peephole) — blocked, then closed by jfecher:
  *"doesn't have enough optimizations to warrant the upkeep."* Small peephole
  passes get rejected on **maintenance-cost** grounds; a new pass must pay rent.
- **PR #10159** (asterite's own `less_than_var` "improvement") — produced *more*
  opcodes/larger circuits because `a<b` and `!(a<b)` stopped sharing a witness.
  Intuition misfires; **measure gates, not opcodes** (matches sparq's own PR-#37
  experience in the `noir-optimisation` skill).
- **PR #8064/#8053** (generic arithmetic-optimization pass) — closed unmerged;
  direct prior art for #8055/#8317.
- **PR #8876** (ownership/assign optimization) — abandoned: loop correctness
  corner cases. **PR #7215** (skip defunctionalization for ACIR) — caused panics.
- **#5791, #2936** (Brillig register/opcode dedup micro-opts) — closed NOT_PLANNED.
- **Timing**: #9820 (loop constraint simplification) explicitly deferred *"after
  1.0 is fully audited"*; the `audit` label period means semantics-touching SSA
  changes with marginal wins face extra resistance. Safe, measured constraint
  reductions are fine.
- **CONTRIBUTING.md**: bans entirely-AI-generated PRs (see §1); typo-level PRs
  closed on sight; issue-first; Conventional Commits v1.0.0 PR titles (scope =
  crate, e.g. `feat(ssa): …`); squash-merge; one approval required; breaking
  changes need DevRel sign-off.

## 5. Measurement methodology + baseline

### 5.1 Metrics (in order of authority)

1. **Backend gate count** — `bb gates -s ultra_honk -b target/<pkg>.json
   [--include_gates_per_opcode]`, `circuit_size` field. Ground truth for proving
   cost. (sparq's `noir-optimisation` skill documents ACIR-opcode counts
   diverging from gates by 5-50×; upstream PR #10159 is the upstream-side
   demonstration.) `bb`/`nargo` are version-coupled — use `bbup`.
2. **ACIR opcode count** — `nargo info [--json]`, per-function ACIR opcodes.
   Cheap iteration proxy; never claim a win from it alone.
3. **Upstream CI** — `.github/workflows/reports.yml` + `benchmark_projects.yml`:
   every PR gets a gates-diff sticky comment (`noir-lang/noir-gates-diff`,
   0.9-quantile summary), Brillig bytecode/trace reports at 3 inliner settings,
   peak-memory reports, and **hard** compile/execution time+memory limits on
   pinned real Aztec circuits. Fork PRs don't get sticky comments — a maintainer
   (or @jeswr on his fork's CI) must surface the diff. SSA snapshot tests will
   fail on circuit-changing PRs and must be regenerated + justified.
4. **Compile time / memory** — wall-clock `nargo compile` and the CI memory
   reports; a circuit-size win must not blow the compile-time budget (#12927's
   O(n²) lesson).

### 5.2 Baseline corpus (measured with HEAD nargo `1.0.0-beta.22+0df14918`, this box)

Corpus = noir's own benchmark programs + sparq's real circuits (copied out of the
sparq repo to `~/noir-optim-workspace/corpus-zk/`, compiled with the HEAD-built
`nargo`). sparq's `zk/` libraries pin `nargo 1.0.0-beta.21`; HEAD is
`beta.22`-dev — one release ahead.

| Package | Source | ACIR opcodes (`main`) |
|---|---|---:|
| `sha512_100_bytes` | noir `test_programs/benchmarks` | 13173 |
| `semaphore_depth_10` | noir `test_programs/benchmarks` | 5699 |
| `bench_eddsa_poseidon` | noir `test_programs/benchmarks` | 4147 |
| `bench_poseidon2_hash_100` | noir `test_programs/benchmarks` | 202 |
| `scan_k2_n64_r8` | sparq `zk/compose` | 22313 |
| `scan_k1_n16_r4` | sparq `zk/compose` | 1743 |
| `filter_f64_d4` | sparq `zk/compose` | 335 |
| `filter_f64` | sparq `zk/compose` | 266 |
| `filter_decimal_i3_f2` | sparq `zk/compose` | 162 |
| `filter_signed_int_d4` | sparq `zk/compose` | 149 |
| `filter_int_d4` | sparq `zk/compose` | 110 |
| `filter_int_d1` | sparq `zk/compose` | 65 |
| `probe_xpath_add_double` | probe bin over sparq `zk/xpath` (`numeric_add_double`) | 396 |
| `probe_xpath_divide_double` | probe bin over sparq `zk/xpath` (`numeric_divide_double`) | 341 |
| `probe_xpath_multiply_double` | probe bin over sparq `zk/xpath` (`numeric_multiply_double`) | 336 |

**Compatibility story:** the whole corpus (pinned to `beta.21`) compiles cleanly
on HEAD `beta.22`-dev — no source changes needed. Caveats: `zk/ieee754` and
`zk/compose/compose_core` and all `zk/xpath` `test_packages/*` are `type = "lib"`
packages, so `nargo info` reports no circuits for them directly; ieee754 is
covered transitively by the `compose` bins, and xpath by the three probe bins
(written for this assessment, in `~/noir-optim-workspace/probes/`, two `pub u64`
witness inputs each — kept out of the sparq repo).

Numbers are ACIR-opcode counts (`nargo info --json`, sum over entry points unless
noted) — the iteration proxy. Gate-level (`bb gates`) baselines should be taken
per-PR on the specific benchmark the PR targets, on the PR's pinned `bb` (this
box's EC2 measurements are non-canonical; upstream CI's gates-diff is the
arbiter).

## 6. Upstream-contribution protocol (binding for every child bead)

1. **Issue-first**: link an existing upstream issue (§4.1) or open one with the
   measured evidence before the PR.
2. **One optimization per PR**, minimal diff, no drive-by refactors; prerequisite
   work as separate PRs; upstream style (`cargo fmt`, clippy, existing pass
   idioms; SSA passes documented with module-level `//!` docs like their
   neighbors).
3. **Draft PR** with the disclosure line: *"This change was developed with
   support from generative AI. I am currently reviewing it before requesting
   maintainer review."* Tag @jeswr for author review; only he flips it to ready.
4. **Conventional-commit title** (`feat(ssa): …` / `fix(acir): …`); squash-merge;
   never force-push after team review begins.
5. **Measured perf analysis in the PR body**: focused benchmark fixture(s) under
   `test_programs/benchmarks/` where appropriate, `bb gates` before/after,
   ACIR-opcode table, compile-time check on a heavy program
   (`sha512_100_bytes`), statement of soundness argument. Regenerate SSA
   snapshots and explain the diffs.
6. **Soundness rules**: every inferred bound a sound over-approximation; no
   global use of branch-local facts; never optimize based on a comparison result
   that the comparison itself enforces (circularity, §2).
7. **Honesty**: numbers only from actual runs; if a win doesn't reproduce on
   `bb gates`, the PR doesn't go up (cf. PR #10159).

## 7. Ranked opportunities

Ranking = (expected circuit-size win on the corpus) × (minimality/low risk of the
diff). Verification status is stated honestly: "verified" means this assessment
read the HEAD code and confirmed the gap; expected wins are *expectations*, not
measurements, unless a number is cited.

| # | Opportunity | Area / evidence | Win basis | Diff size / risk |
|---|---|---|---|---|
| 1 | **Unsigned `div`/`mod` by larger-than-max constant → `0` / `lhs`** (fresh re-derivation of theme c). Verified absent on HEAD: `simplify/binary.rs:144-176` has only `rhs==1`, field-div→mul-by-inverse, and `mod`-by-power-of-two→truncate. Needs a `max(lhs)` bound source: start with the *type* bit width vs constants `≥ 2^bit_size`… which ACIR-gen already rejects — so the useful bound is the local `value_max_num_bits`-style bound (cast/truncate/`and`-derived), scoped exactly like `checked_to_unchecked` | SSA `simplify/binary.rs` + a narrow bound query | jeswr measured 2953→88 gates on the focused fixture (#12781); real-world win on code that masks then divides | Small; low risk; must not duplicate #12781 — coordinate with @jeswr (fresh derivation, different bound source) |
| 2 | **Extend `remove_truncate_after_range_check` to `and`-masked values** (#8628) | SSA `opt/remove_truncate_after_range_check.rs`; maintainer-specified fix | Truncates cost a full euclidean division each (§3.3); upstream wants it | Small; maintainer-blessed; lowest-risk opener |
| 3 | **`to_bits`/`to_radix` with `limb_count == 1` → range constraint** (in-code TODOs `simplify/call.rs:59,79`) | SSA simplify | Removes an array + decomposition per site; win small but the TODO is upstream's own | Tiny; near-zero risk |
| 4 | **Redundant range/`LessThan` constraint elision** (#9463): dominance-aware minimum-bound tracking, same skeleton as `remove_truncate_after_range_check` | SSA new/extended pass | Directly requested; overlaps theme a at its cheapest point | Small-medium; keep it a *pass*, not inline (compile-time lesson §2) |
| 5 | **Push `ArrayGet` through `IfElse` / merge only accessed indices** (#5501) | `ssa/opt/flatten_cfg/value_merger.rs` (`try_merge_only_changed_indices`); precedent PR #11512 | ~2N constraints per merged N-array for one read today; large on array-heavy circuits (xpath corpus is array-heavy) | Medium; correctness-sensitive but issue + precedent exist |
| 6 | **Overflow-check elision when dominated by a later check** (#7161) | SSA | Each elided check saves a range decomposition | Medium; needs the Brillig-side caveat in the issue body |
| 7 | **Value-range / condition-fact tracking in SSA** (#9429; fresh re-derivation of themes a+b) — scope v1 to unconditional single-block facts (jeswr's own soundness boundary), feeding `checked_to_unchecked` + range-check elision | SSA (new analysis; consumers exist) | The general version of rows 1/4; jeswr's #12927 measured −31%/−12%/−22% opcodes on focused fixtures, unchanged on standard circuits | Large-for-this-program; land only after rows 1-4 build trust; coordinate with the open #12927 to avoid competing PRs |
| 8 | **NOT-canonicalization to unlock `IfElse` merging** (TODOs `simplify.rs:343,361`) | SSA simplify / CSE of `not` | Unlocks nested-if merges currently skipped | Small-medium; upstream's own TODO |
| 9 | **Comparison lowering: sign-bit extraction via specialized power-of-two split instead of full euclidean division** (`acir_context/mod.rs:1163-1228`; quotient is 0/1) | ACIR-gen | Every `<`/`>=` pays quotient+remainder range checks today | Medium; **landmine PR #10159 lives exactly here** — witness-sharing effects; `bb gates` before any claim |
| 10 | **Quadratic ACIR expression growth in loops** (#4629; `as_witness` is the manual workaround) | ACIR-gen | High value (quadratic→linear on loop-heavy circuits) | High effort/risk; design-first |
| 11 | **Conditional RAM writes: removable `BoundedVec` capacity assert** (#5027 sub-task identified by TomAFrench) | stdlib/SSA | Slice of a 1.65M-vs-12k constraint gap | Small sub-task of a hard issue |
| 12 | **Investigations (on-ramps)**: #6624 negated-jmpif for ACIR; #6313 inverse-pair stdlib tests; #4972 predicated `array_get` alternative measurement | mixed | Zero-risk credibility builders; may spawn measured PRs | Tiny |

**Explicitly rejected during this assessment (documented so they aren't re-derived):**

- "Redundant remainder range check in `euclidean_division_var`" — intentional;
  in-code comment says the ACVM optimizer removes it when redundant and it is
  *required* for non-power-of-two divisors (`acir_context/mod.rs:~900-910`).
  (Confirmed by `sq-jfkwk`, §10.7, with the mechanism corrected: for a
  power-of-two divisor the duplicate checks land on the *same* witness and the
  ACVM `redundant_range` optimizer deduplicates them.)
- **Row 9's own sketch — "specialize the sign-bit split because the quotient is
  0/1"** — rejected by `sq-jfkwk` (§10.7): the constant-`rhs` bound tightening
  already makes the quotient range check 1 bit, so the specialized path is
  opcode-for-opcode identical to HEAD. Do not re-derive it.
- **Row 8's own TODO — "NOT-canonicalization to unlock `IfElse` merging"** —
  implemented, unit-tested and measured by `sq-seust` (§10.8): the merge fires
  **zero** times across 556 corpus packages, and the NOT-canonicalization half is
  unnecessary anyway (upstream's #6886 removed the stated blocker the same day the
  TODO was written). PARKED, no PR. Do not re-derive.
- "AND/XOR outputs unconstrained vs `redundant_range` assumption is a bug" — it
  is a documented Barretenberg backend contract (issue #1439 +
  `redundant_range.rs:172` comment). A portability question, not a missed
  optimization.
- `x + x → 2x`, `x / x → 1` algebraic rules — no gate win in a field (both are
  one linear/mul op), and `x/x` is unsound at `x = 0` for integer types.
- `x * 2^k → x << k` — backwards on ACIR: multiplication by a constant is a free
  linear term; shifts are the expensive lowering.
- stdlib `lt_fallback` "early exit" and `quicksort` copy-elimination — flattened
  circuits pay both arms (no early exit), and quicksort is unconstrained
  (proof-cost-irrelevant).
- Brillig peephole passes — rejected upstream (PR #11580, #5791, #2936).

## 8. Program sequencing

1. Open with rows 2-3 (tiny, maintainer-blessed/TODO-backed) to establish the
   contribution channel and validate the measurement loop end-to-end (fixture →
   `bb gates` → CI gates-diff).
2. Then row 1 (theme-c re-derivation) and row 4 (#9463) as the first substantive
   circuit-size PRs.
3. Rows 5-6 next; row 7 (the range-analysis flagship) only after the small PRs
   have maintainer trust *and* after checking the fate of jeswr's open #12780/#12927
   (if upstream engages with those, our fresh version may be unnecessary — the
   goal is upstream improvement, not duplicate PRs).
4. Rows 9-11 opportunistically, each gated on a measured `bb gates` win; row 12
   interleaved as cheap goodwill.

Corpus regression check for every PR: recompile the sparq corpus packages (§5.2)
with the patched compiler and diff ACIR opcodes; any unexplained increase is a
stop signal.

## 9. Beads

One bead per ranked opportunity, children referenced to epic `sq-uuvac`:

| Bead | Opportunity (row in §7) |
|---|---|
| `sq-3qwv1` | 1 — div/mod by larger-than-max constant (theme c) |
| `sq-9xhoa` | 2 — `remove_truncate_after_range_check` × `and` masks (#8628) |
| `sq-fwcuo` | 3 — `to_bits`/`to_radix` `limb_count == 1` TODO |
| `sq-rxir8` | 4 — redundant range/`LessThan` elision (#9463) |
| `sq-m3l62` | 5 — ArrayGet through IfElse (#5501) |
| `sq-jthy1` | 6 — dominated overflow-check elision (#7161) |
| `sq-jj3ne` | 7 — value-range/condition-fact tracking v1 (#9429, flagship) |
| `sq-seust` | 8 — NOT canonicalization for IfElse merging |
| `sq-jfkwk` | 9 — comparison lowering power-of-two split (measure-first) |
| `sq-felqr` | 10 — quadratic ACIR expression growth (#4629, design-first) |
| `sq-b0vpc` | 11 — BoundedVec capacity-assert slice (#5027) |
| `sq-eesz3` | 12 — investigation on-ramps (#6624 / #6313 / #4972) |

## 10. FRONT decomposition + program status (2026-07-10)

> 🤖 **SPARQ agent** [FABLE-5] — architect pass for epic `sq-uuvac` (Fable
> collaboration tier). This section reconciles the plan above with the shipped
> state and re-specs the remaining beads for the implementation fleet. Nothing
> above is retracted; where this section disagrees with §8–9, this section wins.

### 10.1 Status (verified against GitHub + `bd` on 2026-07-10)

Steps 1–2 of the §8 sequencing SHIPPED as four upstream draft PRs, all restyled
to the maintainer's binding brevity feedback (2026-07-05):

| PR | Opportunity (§7 row) | Bead | State 2026-07-10 |
|---|---|---|---|
| noir#13263 | 1 — div/mod by oversized constant (theme c) | `sq-3qwv1` done | draft open, `MERGEABLE`, no human review |
| noir#13264 | 2 — truncate after AND mask (#8628) | `sq-9xhoa` in-progress | draft open, `MERGEABLE`, no human review |
| noir#13265 | 3 — single-limb `to_bits`/`to_radix` | `sq-fwcuo` in-progress | draft open, `MERGEABLE`, no human review |
| noir#13266 | 4 — dominating-bound range/`Lt` elision (#9463) | `sq-rxir8` in-progress | draft open, `MERGEABLE`, no human review |

The pending gate on all four is @jeswr's author review (§6 protocol: only he
flips draft → ready). jeswr's original #12780/#12781/#12927 also remain open
with no maintainer engagement — their fate is the coordination pre-flight for
the flagship bead. `sq-uuvac.1` closed as a duplicate of upstream #12794
(already fixed in beta.22). The paper program (`sq-1j5ow` + children) stays
deferred per the maintainer (P3, LOW priority).

**Tracking correction:** the twelve §9 beads referenced this epic only in
prose, so `sq-uuvac` falsely read "2/2 complete — eligible for close" while
eight beads were open. All twelve are now wired as `bd` children of the epic,
plus a new shepherd bead `sq-uuvac.2` (see §10.3). Visibility note: children
under an epic do not surface in `bd ready` — drive this program by curated
waves / enumerating the epic's children directly.

### 10.2 Common acceptance protocol (every remaining implementation bead)

The one load-bearing property of every bead is **circuit-semantics
preservation**: a wrongly elided constraint *under-constrains* the produced
circuit — in a proving system that is a correctness/soundness bug, not a perf
bug. Acceptance is therefore mechanical and identical in shape across beads
(each bead's `bd` acceptance field adds its specifics):

1. `cargo test -p noirc_evaluator` green; SSA snapshot tests regenerated with
   every diff justified in the PR.
2. Differential execution on the focused fixture(s): `nargo execute --force`
   vs `nargo execute --force-brillig` — identical witness/failure behavior.
3. Corpus regression: recompile `~/noir-optim-workspace/corpus-zk` + `probes/`
   with the patched compiler; any unexplained ACIR increase is a stop signal
   (§8).
4. `bb gates -s ultra_honk` before/after on the focused fixture — a win must
   reproduce at gate level or the PR does not go up (§5.1; PR #10159 lesson).
5. Compile-time sanity on `sha512_100_bytes` (the #12927 O(n²) lesson).
6. Upstream etiquette per §6 **plus the maintainer's binding brevity feedback
   (2026-07-04)**: 1–2-line upstream-style code comments; PR body = short
   summary + small perf table + one-line caveat(s) + disclosure line, long
   analysis at most in a collapsed `<details>` block. Draft, @jeswr tagged;
   no agent arms anything — upstream merge is the only "arm".

These criteria gate each change by tests + measurement; they are evidence of
semantics preservation, not a formal proof of it.

### 10.3 Fleet spec for the remaining beads

Each row is mirrored into the bead's `bd` acceptance/notes fields so a fleet
agent can pick it up cold. Worktree = fresh `~/noir-optim-workspace/wt-<bead>`
off noir `master`; target files are pairwise DISJOINT except where a `bd dep`
edge sequences them (§10.4).

| Bead | Tier | Target (noir repo) | One-line scope + risk note |
|---|---|---|---|
| `sq-jthy1` | opus | `ssa/opt/checked_to_unchecked.rs` | elide overflow checks dominated by a later range check (#7161); Brillig failure-point semantics in scope — **DESIGN + FREQUENCY MEASURED, not implemented; a HEAD prerequisite fix must land first; see §10.9** |
| `sq-m3l62` | opus | ~~`ssa/opt/flatten_cfg/value_merger.rs`~~ | ArrayGet through IfElse (#5501); conservative use/alias analysis; precedent PR #11512 — **RE-SPEC REQUIRED BEFORE PICKUP: the target file moved and the target function no longer exists; the #5501 ask is already implemented upstream. See §10.10** |
| `sq-seust` | sonnet | `ssa/ir/dfg/simplify.rs` | NOT canonicalization for IfElse merging; PARK with a findings note if measured neutral (PR #11580 upkeep landmine) — **MEASURED NULL RESULT (0 firings / 556 packages); PARKED per that instruction, no PR; see §10.8** |
| `sq-jfkwk` | sonnet | findings first; `acir/acir_context/mod.rs` only on a win | comparison-lowering experiment; fail-closed on the #10159 witness-sharing landmine; null result acceptable — **NULL RESULT returned, no PR proposed; bead still open, see §10.7** |
| `sq-felqr` | opus | design note on noir#4629 only | quadratic ACIR growth; NO implementation before upstream buy-in — **design note DELIVERED, empirical half + live-thread check PENDING; bead still open, see §10.6** |
| `sq-b0vpc` | sonnet | `noir_stdlib/src/collections/bounded_vec.nr` | only the TomAFrench capacity-assert slice of #5027 — **ALREADY IMPLEMENTED UPSTREAM as draft noir#13314 (@jeswr, 2026-07-10); do NOT re-implement. Two pre-flip review findings outstanding; see §10.11** |
| `sq-eesz3` | sonnet | none (findings-only) | #6624 / #6313 / #4972 measurement comments upstream — **source analysis done, empirical half + live-thread checks PENDING; bead still open, see §10.5** |
| `sq-jj3ne` | opus | new `ssa/ir/dfg/` range-analysis module + its two consumer passes | flagship v1; dep-gated on `sq-jthy1` + `sq-rxir8` (shared consumer files) and on the #12780/#12927 coordination pre-flight |
| `sq-uuvac.2` | haiku | none (existing PR branches only) | shepherd the four open drafts: triage feedback (escalate substantive objections to an opus bead, never rebut directly), rebase on conflict, close sibling beads on merge, refresh the baseline on a new noir pin |

Tier rationale: every bead that *removes* constraints or rewrites merge logic
is opus — under-constraining risk, and per the standing routing rule this
ZK-adjacent soundness-sensitive work goes to Opus rather than cheaper tiers.
Measure-first experiments, the scoped stdlib slice, and findings-only work are
sonnet; pure monitoring is haiku. All upstream PRs are maintainer-gated by
construction (@jeswr author review), so no fleet auto-arm exists anywhere in
this program.

### 10.4 Disjointness + dependency edges

No two beads touch the same noir file except `sq-jj3ne`, whose consumers are
exactly `sq-jthy1`'s pass (`checked_to_unchecked.rs`) and the #13266 elision
pass (`sq-rxir8`) — hence the `bd dep` edges: `sq-jthy1` blocks `sq-jj3ne` and
`sq-rxir8` blocks `sq-jj3ne` (NON-parallel with either). All other beads run
in parallel worktrees with zero textual conflict, and the one-optimization-
per-PR rule (§6) keeps upstream review independence intact.

### 10.5 `sq-eesz3` (row 12, investigation on-ramps) — outcome (2026-07-27)

> 🤖 **SPARQ agent** [OPUS-5]. Full record:
> `noir-investigation-on-ramps-upstream.md` (analysed against noir `e22cd89b`).

Source-level analysis of all three items is complete; the **empirical half was not
run** — the executing environment had no Rust toolchain, `nargo` or `bb`, so there
are no opcode or gate numbers. Two of the three questions were nevertheless largely
settled from the compiler source, and both settled *against* the issue's framing:

| item | outcome | follow-up bead |
|---|---|---|
| #6624 negated jmpif for ACIR | the ACIR guard is load-bearing (`flatten_cfg` merge invariant, tests pin both sides); `Not` is a linear ACIR op and flattening re-creates one anyway ⇒ direct win predicted ≈ 0 | none |
| #6313 inverse-pair stdlib tests | the recomposition half is *predicted* free — linear Horner recomposition, **not measured**, and §4.2 can still overturn it; the decomposition half is a *constraint* (`ToRadix` is `has_side_effects`) and must not be elided ⇒ the requested test needs re-scoping from "optimizes away" to an opcode-count equality | undecided — deferred until §4.2 is measured |
| #4972 predicated `array_get` | the index-offset scheme appears **already implemented** (PR #4971 lineage); only the non-simple-array masking multiplication remains | none |

No win is demonstrated **from the source**, so no implementation bead is spawned
for #6624 or #4972; the #6313 spawn/no-spawn call is **deferred**, not made,
because §4.2 is the thing that decides it. `sq-eesz3` therefore stays **open and
blocked on the empirical half** — its stated deliverable (measurement comments
upstream) is not yet met, so the bead's exit condition is *not* applied here.
Three upstream comments are **drafted but not posted**, pending @jeswr review per
`AGENTS.md` § *Upstream contributions*, and each must first be re-checked against
its live issue thread (none of the three issue bodies was read); the #4972 draft
additionally depends on an unverified inference about the issue's lineage. The
remaining measurement protocol is specified command-by-command in §4 of the record
and needs a box with the noir toolchain.

### 10.6 `sq-felqr` (row 10, quadratic ACIR expression growth, noir#4629) — design note (2026-07-27)

> 🤖 **SPARQ agent** [OPUS-5]. Full record:
> `noir-acir-expression-growth-4629.md` (analysed against noir `e22cd89b`,
> workspace version `1.0.0-beta.25`).

The design note is delivered; the bead stays **open**, blocked on the same
empirical half as `sq-eesz3` (§10.5) — the box had no rustc `1.89.0` (the
workspace pin; only `1.88.0` installed, `rustup` read-only), hence no `nargo`
and no `bb` — and on its own stated exit condition, upstream buy-in on the
issue thread. **No implementation was started and none is proposed**, per the
§10.3 spec for this bead. The #4629 body and thread were **not read** (no
outbound page fetch in that run), so the ask is still the §4.1 paraphrase.

What the source analysis establishes:

| finding | consequence |
|---|---|
| ACIR-gen's `add_var` concatenates expressions with no width bound, and *every* cut site is a consumer that needs a `Witness` rather than an `Expression` — never a size heuristic | the growth is by construction, not a bug in a pass |
| all cut sites funnel through `get_or_create_witness_var`, which rewrites the var in place; `as_witness` is exactly that one call and nothing else | the automation mechanism already exists — the risk is entirely in *where to place the call*, not in plumbing |
| unsigned accumulation is already cut once per iteration by the overflow `RangeCheck` | "quadratic in loops" is narrower than it sounds; the quadratic-by-construction shapes are repeated `assert_eq` on a growing accumulator, brillig-call inputs, and the compile-time/peak-memory cost of building the expression at all |
| the ACVM `CommonSubexpressionOptimizer` postdates the issue: its CSAT intermediate cache is circuit-global, and `MergeExpressionsOptimizer` refunds any intermediate used in only two `AssertZero` opcodes | much of the circuit-size half may already be absorbed — **and an over-eager auto-cut is partially self-healing**, which is the safety argument any heuristic needs |

The reframing the record recommends taking upstream (after measurement) is
that #4629 is plausibly **two** problems: a circuit-size one that ACVM may already
handle, and a compile-time/peak-memory one that ACVM cannot touch by
construction and that shares a root cause with #13046. The blocking first step
is therefore not design but **re-running the issue's own reproducer at HEAD to
confirm the bug still reproduces**; the full protocol is §7 of the record. One
upstream comment is **drafted, not posted**, pending @jeswr review.

### 10.7 `sq-jfkwk` (row 9, comparison lowering power-of-two split) — null result (2026-07-27)

> 🤖 **SPARQ agent** [OPUS-5]. Full record:
> `noir-comparison-lowering-10159.md` (analysed against noir `e22cd89b`,
> workspace version `1.0.0-beta.25`).

The experiment was run at source level and **the sketch is rejected on the
evidence**. No upstream PR is proposed for it and `acir/acir_context/mod.rs` was
not touched, per the §10.3 fail-closed instruction. The bead stays **open**,
blocked on the same empirical half as `sq-eesz3` (§10.5) and `sq-felqr` (§10.6) —
the box had no rustc `1.89.0` (the workspace pin; only `1.88.0` installed,
`rustup` read-only), hence no `nargo` and no `bb` — and, for the witness-sharing
question, on reading the #10159 thread, which was not fetchable in that run.

The evidence base is stronger than the sibling records' because upstream commits
the *expected ACIR* for these lowerings as snapshot tests, so the opcode sequence
is readable without building:

| finding | consequence |
|---|---|
| for constant `rhs`, `euclidean_division_var` already tightens `max_q_bits` to `bit_size - rhs_bits + 1`, which for `more_than_eq_var` is exactly **1** | the "quotient range check" the sketch removes is already a 1-bit range check — i.e. it already *is* the boolean constraint the sketch would emit for `b` |
| the hint's own output range checks and the explicit ones are identical checks on identical witnesses, deduplicated by the ACVM `redundant_range` optimizer | only one range check per output survives; the §7 "explicitly rejected" entry for the redundant remainder check is confirmed, with the mechanism corrected |
| for a power-of-two divisor the `r < rhs` bound constraint degenerates (padding constant `0`) to the *same* range check on the *same* witness, and is deduplicated too | nothing is left to save |
| upstream's committed `lt_u8` snapshot shows the whole comparison as: Brillig hint + `RANGE(q,1)` + `RANGE(r,8)` + one `AssertZero` | the proposed specialization is **opcode-for-opcode identical** to HEAD ⇒ it cannot win gates and can only lose them |
| `less_than_var` returns the *expression* `1 − q`, never a witness, so `a<b`, `a>=b` and `!(a<b)` all share one hint and one pair of range checks, with the negation free | a derived mechanism for the #10159 regression ("stopped sharing a witness"), and the specific risk any future work in that function must be measured against — **hypothesis, the PR thread was not read** |

One **adjacent candidate** was found and is deliberately left unimplemented:
`bound_constraint_with_offset`'s constant-`rhs` fast path is gated on
`try_into_u128()`, which fails at exactly `2^128`, so 128-bit truncations and
`u128` comparisons fall to the general path and pay a redundant second 128-bit
range check on a fresh witness (visible in the committed
`truncate_field_to_128_bits` snapshot). It sits in a different function from the
comparison lowering and does not engage the witness-sharing risk, but it is a
constraint *removal* and therefore under-constraining-sensitive; it must clear the
§10.2 acceptance protocol and the record's §7 measurement protocol before any PR.
One upstream comment is **drafted, not posted**, pending @jeswr review.

### 10.8 `sq-seust` (row 8, NOT-canonicalization for `IfElse` merging) — measured null result, PARKED (2026-07-27)

> 🤖 **SPARQ agent** [OPUS-5]. Full record:
> `noir-not-canonicalization-ifelse-merging.md` (analysed *and measured* against
> noir `e22cd89b`, workspace version `1.0.0-beta.25`).

Unlike §10.5–10.7 this run **was not blocked on tooling**: rustc `1.89.0` installs
cleanly into a writable `RUSTUP_HOME`, so the compiler was built and a release
`nargo` measured over both corpora. That recipe is §8 of the record and **unblocks
the empirical halves of `sq-eesz3`, `sq-felqr` and `sq-jfkwk`** (`bb` is still
absent, so gate counts remain out of reach).

The optimization was **implemented, unit-tested and measured**, then parked on the
evidence, per the bead's own *"PARK with a findings note if measured neutral"*:

| finding | consequence |
|---|---|
| the TODO'd collapse fires **0 times** across 556 packages (518 noir `test_programs/{benchmarks,execution_success}` + 38 sparq `zk/{compose,xpath}`), instrumented with an env-gated per-site counter | no win exists to measure at gate level; ACIR is unchanged — verified against a baseline `nargo` rebuilt from unpatched HEAD, and the shared benchmarks reproduce the §5.2 baselines exactly |
| the sketch's hard half is **unnecessary**: all 259 firings of the two *existing* nested-merge rules matched by plain `ValueId` equality, and the structural `Not` comparison never added a match | NOT-canonicalization/CSE was never the blocker |
| the TODO's stated blocker was removed by upstream **#6886** (jfecher, `not_instructions` memo, *"keeps ids unique which helps simplifications"*) the **same day** the TODO landed in #6875, and `simplify.rs` already folds `!!v => v` | the comment has misinformed readers for ~19 months; correcting it is a 2-line change that should ride along with a substantive PR, not go up alone (§4.2 closes typo-scale PRs) |
| the collapse can only ever match **array/vector-typed** merges — a numeric `IfElse` is lowered to arithmetic by the same function before any outer merge can inspect it | the opportunity is narrower than the TODO suggests; the 259 existing-rule firings all came from 3 nested-array-merge fixtures |
| the shape row 8 targets is `if c { if !c { a } else { b } } else { d }` — dead by construction — and flattening gates nested branches by the **conjunction** `c & c2`, never by `c` or `!c` | zero is the expected count, not an artefact of a thin corpus |

Soundness splits by shape, and only **row 3** (the collapse that reads the
*then*-value) rests on the *at-most-one-condition-true* invariant the shipped
rules assume (`ssa/interpreter/mod.rs:1213-1230`). **Row 4** — the mirrored
*else*-value collapse — does **not**: an outer selected else-branch only proves
the matched inner condition *false*, and since *both* inner conditions may be
false the nested merge is then the zero value, not the arm the rewrite
substitutes (record §2 carries the counterexample). Row 4 is therefore **not
claimed sound and not offered as reconstructable**, and the same gap sits under
upstream's already-shipped row 2 — unchecked from here, captured as follow-up.
The patch does rewrite only *values*, never the outer `else_condition` that
upstream's `do_not_replace_else_condition_with_nested_if_same_then_cond`
regression test pins. It keeps `cargo test -p noirc_evaluator` green (1887
passed, no snapshot moved) with three added tests that **fail on HEAD and pass
patched**, and the AST-fuzzer `arbtest` targets green — but none of that
exercises the both-inner-conditions-false case, so it is not evidence for row 4. It is **not carried in this repo** — no
upstream PR is proposed, and none should be until a program produces the shape.

### 10.9 `sq-jthy1` (row 6, dominated overflow-check elision, noir#7161) — design + measured frequency, NOT implemented (2026-07-28)

> 🤖 **SPARQ agent** [OPUS-5]. Full record:
> `noir-dominated-overflow-elision-7161.md` (analysed *and measured* against noir
> `a352e715`, workspace version `1.0.0-beta.25`).

Using the §10.8 toolchain recipe, the compiler was built, the SSA interpreter was
used as the executable spec, and an instrumented `nargo` counted candidates over
509 `test_programs/execution_success` packages. `bb` is still absent, so **there
are no gate numbers** and the optimization itself was **not written** — hence no
ACIR-opcode delta and no upstream PR.

| finding | consequence |
|---|---|
| the issue's claim holds in ACIR — measured: eliding the first check keeps the failure, moving it from the first `add` to the second — but the **same edit in a Brillig function turns the trap into `Ok(1)`** (wrapping) | the pass must be **ACIR-only**; the issue's "insert a brillig overflow check" is then unnecessary, at the cost of the two runtimes reporting different failure spans |
| three masking shapes measured on the interpreter: a later `sub`, a `mul` by zero, an intervening `truncate` each let the chain re-enter range | the cover needs monotone, non-reducing steps — the elision is not a two-line pattern match |
| separately, the *elided* op must be a checked unsigned **`add`**: ACIR does not wrap, so an underflowing `sub` escapes **downward** to `p - k`, and a later monotone `add` can carry it past the modulus back into range — **losing** the failure rather than moving it (an analytic argument from the ACIR semantics, not a measured row) | v1 excludes `sub` as the elided target on soundness grounds, and `mul` on magnitude grounds — the latter both as the elided target and as a chain step |
| the right representation is **flip the checked op to `unchecked_*`**, never delete a `RangeCheck` — HEAD's `get_value_max_num_bits`/`operand_max_num_bits`/`can_simplify_arithmetic_identity` are already conservative about unchecked ACIR results | the transform inherits the existing invariant machinery instead of re-deriving it |
| **prerequisite:** `checked_to_unchecked`'s private `required_bit_size` lacks the conservative unchecked-arithmetic arm its sibling `get_value_max_num_bits` has; measured at HEAD, it lets the pass flip a checked `u64` add that *can* overflow (execution changes from `Err(Overflow)` to an out-of-range `Ok`) | fix it first, as its own tiny PR — otherwise the elision can remove the very check it depends on. Noir-source reachability on HEAD is **not** demonstrated, so it is filed as a candidate latent bug, not a live one |
| frequency: **501 structural chain candidates — an upper bound**, not elidable checks (≈20% of the 2516 checked unsigned `add`/`mul` ops surviving the pass today), concentrated in **9 of 509** packages, with **0** of the `RangeCheck`-covered variant. The counter tested the structural shape only: it did not evaluate C5, did not fully establish C3, did not apply the add-only C0 restriction, and did not establish that the covering op survives later passes | the pattern is real (unrolled accumulators, multi-term `+` expressions) yet the existing benchmark suite would show zero movement — a focused fixture is mandatory, and the "does it pay rent" question (PR #11580 landmine) is live |
| the counter-risk is that a range check is one of the sites that cuts an accumulating ACIR expression into a witness (cf. §10.6 / noir#4629) | eliding every intermediate check on a long unrolled chain may **grow** the circuit; must be gate-measured before the design is trusted |

The bead stays **open**: the recommendation is a four-step sequence (prerequisite
helper fix → gate-measurement spike on focused fixtures → the ACIR-only v1 pass →
optional extensions), with an explicit stop-and-park after step 2 if no gate win
reproduces. Two questions are for the maintainer/upstream *before* code: whether
the failure-attribution move is acceptable at all, and whether @jeswr's
open #12780/#12927 range analysis subsumes this. Nothing was posted upstream.

### 10.10 `sq-mtolx` (paper P3, gap analysis over the un-examined stages) — code-level half only (2026-07-29)

> 🤖 **SPARQ agent** [OPUS-5]. Full record:
> `noir-optimization-new-opportunities.md` (analysed against noir `8f33502e`).

A child of the paper bead `sq-1j5ow`, not of this epic, but it lands here because
it produces candidate specs for `sq-uuvac` and corrects two of this section's
rows. Ten un-examined pipeline stages were read at code level; **nothing was
measured** (no `nargo`, no `bb`, no build), so the bead's own spawn gate — *"for
each candidate that reproduces a `bb`-gates win via the P1 harness"* — is unmet by
construction and **zero beads were spawned**. Five candidate specs are written up
ready for `bd create`, each blocked on `sq-i50o4`.

| finding | consequence |
|---|---|
| three absorption layers — mandatory full unrolling, post-unroll dominance-aware CSE, and ACVM circuit-global CSE + two-use-intermediate refunding — each void a whole family of textbook optimisations | the classical catalogue is mostly null here *structurally*; surviving candidates concentrate in expensive **lowering formulations** and **N-distinct-constraints-to-one** rewrites that CSE cannot perform |
| the loop family is refuted: LICM's four real hoisting pessimisms are absorbed by post-unroll CSE (the payoff is SSA size, not gates), and #6631/#6629 are moot under full unrolling — unswitching's risk direction is a gate *regression* | issues #10439/#10438 are probably describing IR size, not proving cost — a correction to take upstream once measured |
| `mem2reg` is **not proof-cost-relevant**: reference memory ops are `unreachable!()`/`Err` in ACIR-gen, and the hypothesised post-flatten cross-block run already exists | the area closes; the gate-bearing memory surface is `ArrayGet`/`ArraySet` lowering, a different pass family |
| **`try_merge_only_changed_indices` no longer exists** — removed upstream by PR #8142, file moved to `ssa/ir/dfg/simplify/value_merger.rs`, live tracker **#8145**, and the #5501 ask is implemented in `flatten_cfg::try_optimize_array_set_merge` | **`sq-m3l62` (§10.3) targets a deleted function in a moved file and must be re-spec'd before pickup.** The residual candidate is narrower: the merger emits two identical reads per *unchanged* index |
| `sq-jfkwk`'s comparison-lowering null result (§10.7) **re-confirmed** at the new pin, all three findings; and its flagged adjacent lead is now a code-level candidate — `bound_constraint_with_offset`'s constant fast path is gated on a 128-bit conversion, so every `u128` comparison and every `Field as u128` pays a redundant full-width range check the 8/64-bit paths do not | the strongest genuinely-new candidate, confirmed by upstream's own committed fixtures; it is a *restructuring*, not a constraint removal — but it is a witness-topology change inside the exact function #10159 regressed, so opcode counts are not admissible evidence |
| an independent re-derivation of PR #13263's rule arrived from the `check_u128_mul_overflow` expansion | evidence that #13263 fires on **compiler-generated** `Div`s, not only hand-written ones — relevant to the honest reading of its focused-fixture number. **No bead**; it is the program's own PR |

The record's own headline recommendation is a program-level one: **`sq-i50o4`
(P1) is not merely the paper's reproducibility prerequisite — it gates this entire
candidate register plus four beads already open and blocked on the same missing
capability** (`sq-eesz3`, `sq-felqr`, `sq-jfkwk`, and the gate half of
`sq-jthy1`). Acquiring a `bb` binary in the measurement environment is the single
highest-value next action in the program. Nothing was posted upstream, and no
upstream issue thread was read from this bead — the issue numbers above are
carried from these records unverified, and #5501 is demonstrably superseded.

### 10.11 `sq-b0vpc` (row 11, `BoundedVec` capacity assert, noir#5027) — already shipped upstream as a draft (2026-07-29)

> 🤖 **SPARQ agent** [OPUS-5]. Full record:
> `noir-bounded-vec-capacity-assert-5027.md`.

**Tracking correction.** The bead was picked up cold as an implementation task and
is **already implemented**: draft PR **noir#13314** (*"feat(stdlib): move BoundedVec
push capacity check to unconstrained hint"*, @jeswr, commit `efec45ef`, authored
2026-07-10) touches exactly the §10.3 target file and nothing else. §10.1's status
table predates it and lists only #13263–#13266; the PR is recorded elsewhere in this
repo (`orchestration/start-here.toml`, as the ask behind sparq#1840) but was never
reflected here, so §10.3 read as un-started. It is not. **Do not re-implement it.**

Verified at `master` on 2026-07-29: the PR is still `DRAFT` and unmerged, and
`bounded_vec.nr:182` still carries the constrained assert. The block is @jeswr's
author review (§6), not fleet capacity. No acceptance criterion in §10.2 was run in
that session — no noir checkout, no `nargo`, no `bb` — so nothing below is a
measurement taken by this bead.

Two findings the companion record raises as preconditions for flipping draft → ready:

| finding | why it blocks the flip |
|---|---|
| **No red-on-wrong-answer test.** The existing `#[test(should_fail_with = "push out of bounds")] push_to_full_vector` (`bounded_vec.nr:1353`) still passes with the patch, because the assert still fires during *execution*. It cannot distinguish a constrained check from an unconstrained hint, and the PR adds no test that can | the elision removes the explicit constrained check and ships no regression guard for the property it relies on (whether the array write's own bound dominates is argued below, not established). §10.2's step-2 differential does **not** close the gap: both sides are execution paths, so the unconstrained assert keeps it green under an under-constrained circuit — it is an execution-equivalence check only. The needed test must bypass or mutate honest witness generation and show constraint-system/verifier rejection of an out-of-range index; no such test exists today. §5 of the companion record (2026-08-01, issue #5062) specifies command-by-command only a **prerequisite** of it — the ACIR memory-bound *mechanism* test on a plain-array fixture, with its witness forge, `bb prove`/`bb verify` oracle and the in-range positive control that stops it passing vacuously. The `BoundedVec::push` regression guard itself is **not** specified: §5.6 item 2 records that the `BoundedVec` form returns no verdict as shaped. Both are **unimplemented and unrun**, blocked on `sq-i50o4` |
| **The measured table is unreproducible today.** Its corpus labels/counts are consistent with §5.2 (8 compose + 4 benchmark packages) — but that establishes nothing about whether the commands ran, whether the reported values came from those runs, or which toolchain produced them, so measurement execution and provenance remain **unverified**; and the UltraHonk gate row is the figure §10.8 and §10.10 report the fleet environment could not produce, with `sq-i50o4` not landed | §5.1 makes gates the arbiter and §4.2's #10159 lesson is that opcodes alone mislead. Unreproducible ≠ wrong, but confirmation or a reproducible rerun is a precondition for relying on the gate row, and it goes upstream under a named human author. §6 of the companion record (2026-08-01, issue #5063) works the axis as far as it goes without a toolchain and finds that **only one of the three blockers is actually `sq-i50o4`**: the commit message names no fixture, no `nargo` version and no `bb` version, so under §5.1's `bbup` coupling the row is not re-takeable by *anyone* on *any* box until @jeswr supplies them (unblocked); the fixture it names does exist (sirasistant's `BoundedVec` SIZE=500 comment program in the #5027 thread) but the PR ships it nowhere, contra §6 item 5; and the **"sparq compose corpus (8 packages): 0 delta" row is non-evidential** — all 55 `.nr` files under `zk/` plus the whole dependency closure (`compose_core` → `poseidon` v0.3.0, `sparq_ieee754` v0.11.0) contain zero `BoundedVec`/`.push(` occurrences, so a corpus that never calls the patched function reports 0 delta for a broken patch just as readily. Repairing or dropping that row is likewise unblocked. §5.2's baseline corpus has no `BoundedVec`-exercising package to put in its place |

A third, softer item: the PR justifies the elision by the ACVM's execution-time
`IndexOutOfBounds` error, which is an honest-prover property. The stronger in-tree
argument — that upstream already relies on ACIR's built-in memory-op bound for every
simple-element array index, via `array_index_needs_explicit_oob_check`
(`ssa_gen/context.rs:131-136`) — is the one a maintainer will want. On that reading
the elision is *plausibly* dominated by the array write's own bound; it is **not
certified sound** by this bead, per `sq-qhy4`.

### 10.12 `sq-fwcuo` (row 3, single-limb `to_bits`/`to_radix`) — already shipped upstream as a draft, verified (2026-07-31)

> 🤖 **SPARQ agent** [OPUS-5]. Full record:
> `noir-single-limb-decomposition-13265.md` (verified against noir `master`,
> `d89d99a9`).

**Tracking correction, same shape as §10.11.** The bead was re-dispatched as an
implementation task; it is not one. Row 3 was written and shipped in steps 1–2 as
draft **noir#13265**, which §10.1 already lists — the bead is open because the §6
author-review gate has not been passed, not because code is missing. **Do not
re-implement.** Verified this session: the two TODOs at
`ssa/ir/dfg/simplify/call.rs:59,79` are still present at `master` (so the PR is
unmerged), and all five of its `call.rs` hunks still apply there cleanly — only
that file was checked, not the two snapshot files it also touches.

Nothing was measured — no noir checkout, no `nargo`, no `bb` in this environment,
so no ACIR-opcode delta and no gate count. The record is a source-level
verification of the PR's diff against `master`:

| finding | consequence |
|---|---|
| the rewrite's equivalence holds on every path read: `radix_le_decompose` at one limb is exactly `RANGE(w, log2 radix)` + `w == input`; `Instruction::Cast` is a constraint-free alias in ACIR-gen; and the reused failure string is character-identical to the one **both** runtimes emit (ACIR `generated_acir/mod.rs:422-424`, ACVM Brillig `black_box.rs:345-349`) | the PR is well-constructed; this bead's review is confirmatory, and per `sq-qhy4` still **not** a soundness certification |
| predication is preserved: `flatten_cfg`'s `RangeCheck` arm rewrites the operand to `value * condition` **and** `map_value`s the original id (issue #8617), so the following `Cast` resolves to the predicated value — while the un-rewritten call was predicated by zeroing its argument | the predicated single-limb case, the one shape a reviewer will probe, is sound by the same mechanism on both sides |
| the power-of-two guard costs nothing in ACIR — `radix_decompose` itself `assert!`s `is_power_of_two` — so the declined cases are Brillig-only, exactly as the PR's two decline tests are written | row 3's stated risk ("non-power-of-two radix cannot be a pure range check") is fully retired |
| **the failure path has no live test**: the fixture's out-of-range input is only decomposed under a `false` predicate, so deleting the emitted `RangeCheck` leaves every shipped test green | the red-on-wrong-answer guard is missing — the one actionable precondition before the draft → ready flip, and cheap (an `execution_failure` fixture) |
| the fixture lands in `execution_success`, not `test_programs/benchmarks/`, so no committed number moves if the rule regresses; and the removed recomposition `AssertZero` is the two-term shape the ACVM `MergeExpressionsOptimizer` refunds (§10.6) | any opcode delta counted from SSA/ACIR-gen is an **upper bound** on the post-ACVM one — measure after ACVM, with `bb` as arbiter (§5.1) |
| frequency: `remove_bit_shifts` itself generates `to_le_bits(v) -> [u1; 1]` when the shift exponent's max bit width is 1 (`remove_bit_shifts.rs:347-363`, its committed snapshot at `:698`), and at that site the emitted range check is itself removed as already-implied (`simplify.rs:281-284`) — the decomposition goes away with nothing emitted in its place | the rule fires on **compiler-generated** IR, not only hand-written single-limb calls — the same evidence shape §10.10 found for #13263. Corpus frequency is still unmeasured |

The bead stays **open**, blocked on the §6 author review and on `sq-i50o4` (a `bb`
binary) like the rest of the program. Nothing was posted upstream, and the PR's
body and review thread were not read — only its diff.

### 10.13 `sq-9xhoa` (row 2, truncate after an `and` mask) — already shipped upstream as a draft, verified (2026-08-01)

> 🤖 **SPARQ agent** [OPUS-5]. Full record:
> `noir-truncate-after-and-mask-8628.md` (verified against noir `master`,
> `d89d99a9`).

**Tracking correction, same shape as §10.11 and §10.12.** The bead was
re-dispatched as an implementation task; it is not one. Row 2 (#8628) was written
and shipped in steps 1–2 as draft **noir#13264**, which §10.1 already lists — the
bead is open because the §6 author-review gate has not been passed, not because
code is missing. **Do not re-implement.** Verified this session: `master`'s
`remove_truncate_after_range_check.rs` still has only its three original `match`
arms (so the PR is unmerged), and the whole diff re-applies to `master` with no
offset and no fuzz. Unlike #13265 it touches **no existing snapshot file**, so it
carries no rebase hazard.

Nothing was measured or executed — no `nargo`, no `bb`, no built compiler — so no
ACIR-opcode delta and no gate count. The record is a source-level verification of
the PR's diff against `master`:

| finding | consequence |
|---|---|
| the bound is sound and the arm's dominance question is **vacuous**: `mask.num_bits()` is the value's bit length (`field_element.rs:414-423`, pinned by the upstream proptest `num_bits_agrees_with_ilog2`), and the bound is attached to the `and`'s *own result*, which by SSA dominance already dominates every use — so the pass's `previous_block` clearing heuristic can only cost a missed rewrite here, never an unsound one | stronger than the pre-existing `RangeCheck` arm, whose validity genuinely leans on that heuristic. Per `sq-qhy4` still **not** a soundness certification |
| the tests are non-vacuous (source-derived, not executed): deleting the arm reddens both positive snapshots; computing the bound as popcount instead of bit length reddens the `mask = 0b101, truncate to 2 bits` test; flipping the `<=` reddens the `507` test; and the fixture's `kept` case bites at execution because its author picked an input where masked (`417`) and truncated (`161`) differ | the red-on-wrong-answer gap that blocks #13265 (§10.12) is **already closed** here |
| **the one real gap — signed types.** The `2^n - 1` → `Truncate` canonicalization in `simplify/binary.rs:243` is guarded by `is_unsigned()`, so on a signed type an `and` by a `2^n - 1` mask survives to the pass and fires the new arm — a mask class unreachable on unsigned types. All seven unit tests and all four fixture cases are `u64`/`u32` | the single uniquely-reachable case has zero coverage. A few `i32`/`i64` SSA unit tests close it; cheap, and worth landing on the PR branch |
| **placement question for the maintainer**: neither `get_value_max_num_bits` (`dfg.rs:574-634`, no `And` arm) nor the `Truncate` rule in `simplify.rs:210-262` covers this today, so the PR is not redundant — but that `Truncate` rule already reasons about `Div` **by a constant** in exactly this shape, and `get_value_max_num_bits` also backs the `RangeCheck` removal at `simplify.rs:281-284` | teaching the bound to `get_value_max_num_bits` instead would fire everywhere rather than at pipeline position `ssa/mod.rs:395`, and would drop redundant **range checks** on masked values for free. jfecher's in-issue direction was the pass, so this is a question to raise, not a defect — and it is row 7 (`sq-jj3ne`) territory |
| the cost claim holds structurally: `convert_ssa_truncate` (`acir/mod.rs:856-892`) has **no early-out** on known bit width and always lowers to `truncate_var` → `euclidean_division_var` (`acir_context/mod.rs:1165-1179`) | each removal really does avoid a whole euclidean-division lowering; its *size* stays unquantified pending `bb` |
| frequency is **weak-negative and unmeasured**: the diff updates no existing snapshot, and the idiomatic `& 0xFF`/`& 0xFFFF` masks are precisely the ones already canonicalized to truncations, so the arm's reachable population on unsigned types is the unusual non-`2^n - 1` masks | contrast §10.12, where the rule demonstrably fired on compiler-generated IR. The #11580 "does it pay rent" question is open, and the fixture sits in `execution_success`, not `test_programs/benchmarks/`, so no committed number moves if the rule regresses |

The bead stays **open**, blocked on the §6 author review and on `sq-i50o4` like
the rest of the program. Nothing was posted upstream, and the PR's body and review
thread were not read — only its diff.
