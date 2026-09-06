# Noir upstream investigation on-ramps — #6624 / #6313 / #4972 (sq-eesz3)

**Bead:** `sq-eesz3` (epic `sq-uuvac`, `noir-optimization-program.md` §7 row 12) · **Status:**
**PROVISIONAL — bead NOT complete**: source-level analysis is complete for all three items, but the
bead's deliverable (measurement comments upstream) is not met — **empirical measurement NOT RUN**
and **no live issue thread read** (see §0), so the bead stays open, blocked on the empirical half
(§4) · **Upstream comments DRAFTED, NOT posted** — awaiting @jeswr review per `AGENTS.md`
§ *Upstream contributions* · **Author:** SPARQ agent 🤖 [OPUS-5] · **Date:** 2026-07-27

Analysed against `noir-lang/noir` @ `e22cd89b` (2026-07-24). Every line citation below is at that
commit. Companion record: `noir-optimization-program.md` (§4.1 issue table, §10 fleet spec).

## 0. What this record is, and what it is not

`sq-eesz3` is a **findings-only** bead: three cheap, maintainer-suggested investigations whose
deliverable is a measurement comment upstream, plus a follow-up bead *only where a win is
demonstrated*. It was executed in an environment that turned out to be **static-analysis-only**:

| capability | available | consequence |
|---|---|---|
| noir source at HEAD | yes (shallow clone) | full source analysis possible |
| Rust toolchain / `cargo` | **no** (read-only rustup; pinned channel absent) | `noirc` cannot be built |
| `nargo`, `bb` | **no** | no ACIR opcode counts, no `bb gates` |
| noir issue/PR bodies | **not fetched** | issue *asks* are taken from `noir-optimization-program.md` §4.1, not re-read from the live issues |

So this record establishes what is **derivable from the compiler source** — which turned out to be
the larger half of all three questions, and in two cases changes what the measurement should even
be — and then specifies the pending empirical half precisely enough to be run mechanically
(§4). **Nothing here is a measured claim.** Where a cost is stated, it is an argument from the
lowering code, and is labelled as such. Per the program's own doctrine (§2 of the program record,
and the PR #10159 lesson: *measure gates, not intuition*), none of the verdicts below may be
asserted upstream as measurements, and the drafted comments in §1–§3 say so in their own text.

Consequence for the bead's exit condition: **it is not reached, and this record does not apply
it.** `sq-eesz3`'s deliverable is a measurement comment upstream; nothing was measured and nothing
was posted, so the bead remains **open and blocked on the empirical half** (§4) and on reading the
three live issue threads. What *can* be concluded now is narrower: from the source alone, no win is
visible for #6624 or #4972, so neither warrants an implementation bead **on present evidence** —
a judgement that must survive the live-thread check before it is acted on. For #6313 the
spawn/no-spawn decision is explicitly **deferred**, because §4.2 is what decides it.

---

## 1. noir#6624 — is `check_for_negated_jmpif_condition` worth enabling for ACIR?

**Ask (program record §4.1):** pure investigation — does this SSA simplification benefit ACIR?

### 1.1 The transformation is *deliberately* disabled for ACIR, and the reason is in-code

`check_for_negated_jmpif_condition` (`compiler/noirc_evaluator/src/ssa/opt/simplify_cfg.rs:314`)
rewrites `jmpif (not c) then: A, else: B` into `jmpif c then: B, else: A`. Its very first
statement is an early return for ACIR functions (`:319-336`), carrying the rationale: swapping the
arms can produce a CFG where the two branches re-converge **in the `then` block**, and
`flatten_cfg` assumes a merge happens in the `else` block or in a third block. The comment cites
the review discussion on PR #5891. Both behaviours are pinned by tests —
`swap_negated_jmpif_branches_in_brillig` (`:645`) and
`does_not_swap_negated_jmpif_branches_in_acir` (`:684`).

So the issue's framing matters: this is **not a flag flip**. "Enabling it for ACIR" means first
relaxing a structural invariant of `flatten_cfg`, or making the swap conditional on the resulting
CFG shape. That is a real change with a real failure mode (a broken flatten is a miscompile, not a
slowdown), which puts it in a different risk class than the issue's "investigation" label suggests.

### 1.2 The win it targets appears to be ~nil in ACIR — three independent reasons

1. **`Not` is a linear operation in ACIR.** `not_var`
   (`compiler/noirc_evaluator/src/acir/acir_context/mod.rs:767-777`) lowers to `max - x`, a
   constant minus the variable. It folds into the surrounding CSAT expression; on its own it
   creates neither a witness nor a gate. The instruction the swap removes is among the cheapest
   things the ACIR IR has. (This is the same asymmetry that put `x * 2^k → x << k` on the program
   record's rejected list: cheap-in-a-normal-IR and cheap-in-ACIR are different orderings.)
2. **`flatten_cfg` re-introduces a `not` regardless.** Merging arms uses
   `c * then_arg + !c * else_arg` (module doc, `ssa/opt/flatten_cfg.rs:80-100`), with the
   else-condition derived as `NOT(then_condition)` (`:808`). The same module doc states the pass
   "is known to produce some extra instructions which may go unused (usually 'Not')" (`:38`).
   Swapping the arms changes *which* side needs the negation; it does not remove the need for one.
3. **Ordering leaves almost no window.** In the primary pipeline, `simplify_cfg` runs immediately
   before `flatten_cfg` (`ssa/mod.rs:311-316`), so for an ACIR function every `jmpif` this pass
   could rewrite is about to be flattened away. What survives the swap is an SSA-level instruction
   count, which the later DIE pass already cleans — i.e. a compile-time effect at most, not a
   circuit-size one.

### 1.3 The honest residual

The above predicts a **direct** delta of zero. It does *not* rule out a second-order effect: a
swapped condition changes which values CSE and the shape-sensitive merge-collapse heuristics in
`flatten_cfg` (`try_collapse_merge`, `:808`) observe, and witness-sharing second-order effects are
exactly what made PR #10159 *regress* while looking like an improvement. So the prediction is
falsifiable only by measurement (§4.1), and the drafted comment says precisely that.

### 1.4 Verdict

**No PR from this bead.** Recommended upstream action is a comment that (a) records the blocker as
the `flatten_cfg` invariant rather than a missing feature, (b) gives the cost model above with line
pointers, and (c) offers to run the experiment. Draft:

> **DRAFT — not posted.** Requires @jeswr review; must be re-checked against the live issue thread
> (this analysis did not read the issue body) and re-run against HEAD before posting.
>
> > 🤖 Posted by an autonomous agent (a SPARQ agent) operating on @jeswr's behalf.
>
> Looking at this at `e22cd89b`: the ACIR early-return in `check_for_negated_jmpif_condition`
> (`simplify_cfg.rs:319-336`) is load-bearing rather than incidental — swapping the arms can make
> them re-converge in the `then` block, which `flatten_cfg` does not expect (per the PR #5891
> discussion the comment cites), and both sides are pinned by tests (`:645`, `:684`).
>
> Reading the lowering, the direct win for ACIR looks like it should be close to zero: `Not` lowers
> to `max - x` (`acir_context/mod.rs:767`), a linear term that folds into the surrounding
> expression; `flatten_cfg` derives the else-condition as `NOT(then_condition)` for the
> `c*then + !c*else` merge anyway (and its module doc notes it routinely emits unused `Not`s); and
> `simplify_cfg` runs immediately before `flatten_cfg` for ACIR, so the rewritten `jmpif` is about
> to disappear. That argues the remaining effect is SSA instruction count (compile time), not
> circuit size.
>
> That is an argument from the source, **not a measurement** — and second-order witness-sharing
> effects have gone the wrong way here before (#10159). Happy to run the experiment (drop the ACIR
> guard behind a temporary flag → test suite for flatten ICEs → `bb gates -s ultra_honk` diff on
> `test_programs/benchmarks/*`) and post the numbers, if that is useful for deciding whether to
> keep the issue open.

---

## 2. noir#6313 — do near-inverse stdlib pairs optimize away?

**Ask (program record §3.5, §4.1):** add tests verifying that near-inverse pairs
(`to_be_bytes`/`from_be_bytes` etc.) optimize away; fix them if they don't.

### 2.1 The two halves of the round trip have opposite cost structures

- **`from_le_bytes` / `from_be_bytes`** (`noir_stdlib/src/field/mod.nr:211`, `:233`) are a Horner
  recomposition `Σ bytes[i] · 256^i` over constant-index reads: casts, multiplications by
  compile-time constants, and additions — **all linear in ACIR**. On top of an existing
  decomposition this half should cost essentially nothing.
- **`to_le_bytes` / `to_be_bytes`** (`:94`, `:131`) are not a computation but a **constraint**.
  `to_*_radix(256)` lowers to `radix_le_decompose`
  (`acir/acir_context/generated_acir/mod.rs:381-430`), which emits one `range_constraint` per limb
  **plus** an `assert_is_zero(input − Σ limb·256^i)` composition constraint. The stdlib then
  appends, in the constrained runtime only, an N-iteration canonicality loop against the modulus
  bytes (`field/mod.nr:100-118`, `:137-155`).

### 2.2 The decomposition is ineligible for elimination — by design, and correctly

`Intrinsic::ToBits`/`ToRadix` are classified `has_side_effects() == true`
(`ssa/ir/instruction.rs:199`) with the comment *"these apply a constraint that the input must fit
into a specified number of limbs"*, and `Purity::PureWithPredicate` (`:255`), which permits
deduplication but not deletion. So an unused round trip does **not** vanish — and should not: for
`N` below the modulus width, `x.to_be_bytes::<N>()` carries a genuine failure condition (`x` must
fit in `N` bytes). Removing it would drop a constraint the program's failure semantics depend on —
the under-constraining failure mode the program record calls out as a correctness bug, not a perf
bug.

### 2.3 Therefore the test the issue asks for needs re-scoping

`compile_success_empty` is the harness venue that asserts an empty circuit
(`test_programs/README.md`; `tooling/nargo_cli/tests/execute.rs:252`). It is the natural place to
put a "this optimizes away" test — but for a round trip on a **witness** input it cannot pass, for
the reason in §2.2, and for a **constant** input it passes by comptime folding alone, which makes
the test vacuous (it would stay green even if every optimization under test were deleted).

The assertion that is both true and non-vacuous is the narrower one:

> the round trip `Field::from_be_bytes::<N>(x.to_be_bytes::<N>())` costs the **same** ACIR opcodes
> as the bare `x.to_be_bytes::<N>()` — i.e. the recomposition half is free —

which belongs in an opcode-count fixture (the `gates_report` / benchmark path), not in
`compile_success_empty`. This is a genuinely useful thing to report upstream: it says the issue is
worth keeping, but that its title's phrasing ("optimize away") is unachievable as stated for the
decomposition half, and names what should be pinned instead.

### 2.4 Where a real win would remain

Not in the recomposition (already free by the argument above) but in the decomposition:

- the N-iteration **canonicality loop** — for `N` equal to the modulus byte width it is trivially
  satisfiable for every field element, but the compiler has no modulus-width fact with which to
  discharge it. That fact is exactly what the program record's row 7 value-range analysis
  (`sq-jj3ne`) would supply;
- the **single-limb** case, which is upstream's own in-code TODO (`simplify/call.rs`) and already
  owned by row 3 / `sq-fwcuo` (upstream draft #13265).

Both are already-owned beads; this bead adds no new one.

### 2.5 Verdict

**No PR from this bead.** The follow-up-bead decision is **deferred, not made**: §4.2 must be run
first, because this is the one item of the three where the measurement could still change the
answer (if the recomposition turns out *not* to be free, that is a real finding and a real PR).
Draft comment:

> **DRAFT — not posted.** Requires @jeswr review; must be re-checked against the live issue thread
> and re-run against HEAD before posting.
>
> > 🤖 Posted by an autonomous agent (a SPARQ agent) operating on @jeswr's behalf.
>
> A note on scoping this, from reading the lowering at `e22cd89b`. The two halves of
> `from_be_bytes(to_be_bytes(x))` behave very differently:
>
> - `from_*_bytes` is `Σ bytes[i]·256^i` — casts, multiplies by constants, adds, all linear in
>   ACIR, so it should add ~nothing on top of the decomposition.
> - `to_*_bytes` is a *constraint*, not a computation: `radix_le_decompose`
>   (`generated_acir/mod.rs:381`) emits a range constraint per limb plus the composition
>   `assert_is_zero`, and the stdlib adds an N-iteration canonicality loop.
>
> `ToRadix`/`ToBits` are `has_side_effects() == true` (`instruction.rs:199`) exactly because they
> constrain the input to fit in N limbs, so an unused round trip is not DIE-able — and for
> `N` < modulus width it shouldn't be, since `to_be_bytes::<N>` is a real failure condition.
>
> So a `compile_success_empty` test would be vacuous for a constant input and impossible for a
> witness input. The assertion that seems both true and worth pinning is *"the round trip costs the
> same opcodes as the bare decomposition"* — i.e. the recomposition adds nothing — as an
> opcode-count fixture. Would a test in that shape address what you had in mind? Happy to measure
> the opcode counts first and report them here either way.

---

## 3. noir#4972 — the index-offset alternative for predicated `array_get`

**Ask (program record §4.1):** measure TomAFrench's suggested index-offset alternative ACIR-gen for
predicated `array_get`, versus the quadratic-expression form.

### 3.1 The suggested scheme appears to be already implemented at HEAD

`acir/arrays.rs:97-99` documents the predicated index as exactly
`predicate_index = predicate * index + (1 - predicate) * offset`, and
`convert_array_operation_inputs` (`:485-524`) implements it in **one** multiplication rather than
two:

- `get_flattened_index` (`:1202`) already returns `raw_index * predicate` when the index is not
  statically safe (`:1237-1241`);
- the offset bias is then added as `offset * (1 - predicate)` (`:508-517`), where `offset` is a
  **compile-time constant** produced by `compute_offset` (`:452-471`) — so that term is linear;
- the in-code comment states the intent verbatim: this yields `raw_index` when the predicate is 1
  and `offset` when it is 0 *"without a second predicate multiplication"* (`:508-510`).

`compute_offset`'s doc comment cites `https://github.com/noir-lang/noir/pull/4971` (`:449`) — the
PR that issue #4972 immediately follows by number. **Inference, not verified:** #4972 reads as the
follow-up issue to that PR, and the implementation now in tree looks like the thing it asked to be
measured. This inference is unverified because the issue body was not read in this environment
(§0), and it is the first thing to check before posting anything.

### 3.2 The residual — where a multiplication is still paid

Two narrow cases remain, and they are the only measurable question left:

1. **`compute_offset` returns `None` for non-simple arrays** — it applies only when the instruction
   has a single result *and* `array_has_constant_element_size(..) == Some(1)` (`:458-459`). For
   anything else there is no type-compatible offset, the disabled-branch index collapses to `0`,
   and the doc's *"fallback costing one multiplication in the worst case"* (`:447`) applies.
2. **`apply_index_side_effects`** (`:719-758`) emits that fallback: one masking
   `mul(value, predicate)` per scalar read, skipped only when element 0's type is width-compatible
   with the read's result type (`:731-736`).

So the useful question is no longer "index-offset vs quadratic expression" — that looks settled —
but *"how often does `compute_offset` decline on real circuits, and what does the masking
multiplication cost there?"*

### 3.3 Verdict

**No PR from this bead.** This is the cheapest and most credibility-positive of the three, because
the comment is *"your suggestion appears to have landed; here are the code pointers; is this issue
stale, or is the non-simple-array residual what remains?"* Draft:

> **DRAFT — not posted.** Requires @jeswr review. **Blocking pre-check:** confirm from the issue
> body that #4972 is indeed the follow-up to PR #4971 and that the scheme described below is the
> one it proposed — this draft is built on a source-level inference, not on the issue text.
>
> > 🤖 Posted by an autonomous agent (a SPARQ agent) operating on @jeswr's behalf.
>
> At `e22cd89b` this looks like it may already be in tree. `arrays.rs:97` documents the predicated
> index as `predicate * index + (1 - predicate) * offset`, and
> `convert_array_operation_inputs` (`:508-517`) builds it with a single multiplication —
> `get_flattened_index` already returns `raw_index * predicate` (`:1237`), and the offset term is
> constant-folded, with the comment noting it avoids "a second predicate multiplication".
> `compute_offset` (`:452`) cites PR #4971.
>
> What still costs a multiplication is the case where `compute_offset` declines — it only handles
> single-result reads on arrays with constant element size 1 — and `apply_index_side_effects`
> (`:738-744`) falls back to masking the read value by the predicate. If that residual is what is
> left of this issue, I can measure how often the offset path declines across the benchmark
> programs and what the masking costs; otherwise this may be closeable.

---

## 4. The pending empirical half — exact protocol

Requires a box with a noir toolchain (`cargo` + `nargo` + `bb`), which this environment did not
have (§0). Baseline for all three: noir @ `e22cd89b`, `bb gates -s ultra_honk`, and the program
record's §10.2 acceptance protocol (differential `nargo execute --force` vs `--force-brillig`,
corpus regression, compile-time sanity on `sha512_100_bytes`).

### 4.1 #6624

1. Delete the ACIR early return (`simplify_cfg.rs:319-336`) behind a temporary flag; delete/adjust
   `does_not_swap_negated_jmpif_branches_in_acir`.
2. `cargo test -p noirc_evaluator` and the `test_programs` suite. **Any flatten-related ICE or
   snapshot break confirms the invariant is still load-bearing** — that alone is a reportable
   finding and ends the experiment.
3. If clean: `bb gates -s ultra_honk` before/after on `test_programs/benchmarks/*` plus a
   branch-heavy fixture. **Decision rule:** report a win only if gates strictly decrease on at
   least one benchmark and increase on none; anything else is reported as a null result (which
   §1.2 predicts).

### 4.2 #6313

1. Fixture A: `fn main(x: Field) -> pub Field { x.to_be_bytes::<32>()[0] as Field }` (bare
   decomposition, byte kept live). Fixture B: the same plus
   `Field::from_be_bytes::<32>(x.to_be_bytes::<32>())` fed to the return value.
2. Compare ACIR opcode counts (`nargo info`) and `bb gates`. **Expected (§2.1): B − A ≈ 0.**
3. Repeat for `N = 8` and for `to_be_bits`/`from_be_bits` where an inverse exists.
4. If B − A is materially positive, that is the real finding and the basis for a follow-up bead; if
   ~0, land the §2.3 opcode-count fixture as the test the issue asked for, re-scoped.

### 4.3 #4972

1. Instrument `compute_offset` (`:452`) with a counter for `Some`/`None` over `test_programs`
   compilation, to quantify how often the offset path declines.
2. On the declining programs, count the `apply_index_side_effects` masking multiplications and
   diff gates against a build where the offset path is (unsoundly, for measurement only) forced —
   an upper bound on the remaining win, not a candidate patch.

---

## 5. What is *not* established by this record

- No gate count, opcode count, or compile-time number anywhere — nothing was executed (§0).
- The three issue bodies were not re-read; asks are quoted from `noir-optimization-program.md`
  §4.1. Every drafted comment must be re-checked against its live thread before posting, and the
  #4972 draft additionally depends on an unverified inference (§3.1).
- The cost arguments are read off the lowering code; they are not proofs of semantics preservation
  and are not immune to the second-order witness-sharing effects that reversed PR #10159.
- Nothing has been posted upstream, and per `AGENTS.md` § *Upstream contributions* nothing may be
  until @jeswr reviews these drafts.
