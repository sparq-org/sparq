# NOT-canonicalization to unlock `IfElse` merging — noir experiment record (sq-seust)

**Bead:** `sq-seust` (epic `sq-uuvac`, `noir-optimization-program.md` §7 row 8, §10.3 *"`ssa/ir/dfg/simplify.rs`;
NOT canonicalization for IfElse merging; PARK with a findings note if measured neutral (PR #11580 upkeep
landmine)"*) · **Status:** **implemented, tested and MEASURED — the optimization is a NULL RESULT on both
corpora, so the bead's own PARK instruction applies.** No upstream PR is proposed · **Author:** SPARQ agent 🤖
[OPUS-5] · **Date:** 2026-07-27

Analysed and measured against `noir-lang/noir` @ `e22cd89be53c37393eed6520a2001193dd723349` (2026-07-24,
workspace version `1.0.0-beta.25`), in a throwaway clone outside this repo. Every line citation is at that
commit. Companion records: `noir-optimization-program.md` (§3.2 SSA pass map, §4.2 landmine list, §10 fleet
spec), `noir-comparison-lowering-10159.md`, `noir-acir-expression-growth-4629.md`,
`noir-investigation-on-ramps-upstream.md`.

## 0. What this record is, and what it is not

Unlike the three sibling findings-only records (§10.5–10.7 of the program record), which were all blocked on
"no Rust toolchain on the box", this run **built the compiler and measured**. The environment recipe that
unblocked it is in §8 and applies to those beads too.

What was actually done: the TODO'd optimization was **implemented**, given three unit tests that **fail on
HEAD and pass with the patch**, and then instrumented to count how often it fires while compiling 556
packages. It fires **zero** times. The verdict below follows from that count, not from taste.

Environment, stated up front because it bounds every claim:

| capability | available | consequence |
|---|---|---|
| noir source at HEAD + full git history | yes | source analysis and archaeology of the TODO's origin |
| rustc `1.89.0` (the workspace pin) | yes — installed into a writable `RUSTUP_HOME` (§8) | compiler builds; `cargo test -p noirc_evaluator` runs |
| `nargo` built from the patched tree | yes (release) | **ACIR opcode counts and firing counts on a real corpus** |
| `bb` | **no** | **no gate counts.** No claim here is a gate measurement — but see §6: nothing fires, so there is no win to gate-check |
| upstream issue/PR threads | PR #6875 / #6886 read **from git history**, not from the web threads; no live thread was fetched | maintainer intent is inferred from commit + code comments |

## 1. The ask

`compiler/noirc_evaluator/src/ssa/ir/dfg/simplify.rs:343-344` and `:361-362` carry the same TODO, twice:

```rust
// TODO: We could check to see if `then_condition == inner_else_condition`
// but we run into issues with duplicate NOT instructions having distinct ValueIds.
```

The bead's sketch: either CSE the `Not` instructions, or match structurally on
`Not(then_condition) == inner_condition`, so that `IfElse(c, IfElse(!c, a, b), d)` collapses.
**WIN:** one fewer merge per occurrence. **Frequency: unknown — measure before claiming value.**

## 2. What the instruction means (the invariant the rewrite needs)

`Instruction::IfElse { then_condition, then_value, else_condition, else_value }`
(`ssa/ir/instruction.rs:417-431`) merges two branch values. The load-bearing invariant is *not*
`else_condition == !then_condition`; the SSA interpreter — the executable spec — says so explicitly
(`ssa/interpreter/mod.rs:1213-1230`):

- **at most one** of the two conditions is true (both true ⇒ `InterpreterError::DoubleTrueIfElse`);
- **both false is legal** — flattening gates a nested branch by the conjunction of all enclosing
  conditions, so `then = outer & a`, `else = outer & !a` are both false when `outer` is false;
- when both are false the result is the zero/uninitialized value, matching the arithmetic decomposition
  `then_condition * then_value + else_condition * else_value`.

The four nested-merge collapses do **not** all follow from *at-most-one*. The two that read the
**then**-value (rows 1 and 3) do; the two that read the **else**-value (rows 2 and 4) need a strictly
stronger premise, spelled out after the table:

| # | shape | when it applies | result | on HEAD |
|---|---|---|---|---|
| 1 | inner merge in the **then**-value, `then_condition == inner_then_condition` | outer then-branch live ⇒ `inner_then_condition` true ⇒ inner then-branch live | take `inner_then_value` | **shipped** |
| 2 | inner merge in the **else**-value, `then_condition == inner_then_condition` | outer else-branch live ⇒ `then_condition` false ⇒ inner then-branch dead — which does **not** make the inner else-branch live (gap below) | take `inner_else_value` | **shipped** |
| 3 | inner merge in the **then**-value, `then_condition == inner_else_condition` | outer then-branch live ⇒ `inner_else_condition` true ⇒ inner *else*-branch live | take `inner_else_value` | **the TODO** |
| 4 | inner merge in the **else**-value, `then_condition == inner_else_condition` | outer else-branch live ⇒ inner else-branch dead — same gap as row 2 | take `inner_then_value` | **the TODO** |

**Rows 1 and 3 are sound under *at-most-one* alone.** The outer then-branch being live makes the *matched*
inner condition **true**, so the inner merge provably evaluates to exactly the arm the rewrite substitutes.

**Rows 2 and 4 are the mirror image, and the argument does not mirror.** There the outer *else*-branch being
live only makes the matched inner condition **false**, which kills one inner arm without electing the other —
and per the second bullet above, *both false is legal*, in which case the inner merge is the
zero/uninitialized value rather than either arm. Concretely for row 4: outer `then_condition = false`,
`else_condition = true`, `inner_else_condition = then_condition = false`, and `inner_then_condition = false`
too. The nested merge yields zero; the documented rewrite substitutes `inner_then_value`. Row 2 is symmetric
(`inner_then_condition = then_condition = false` with `inner_else_condition` also false yields zero, not
`inner_else_value`).

What rows 2 and 4 actually need is the strictly stronger **exactly-one** premise *on the inner merge* —
`inner_then_condition | inner_else_condition` must hold whenever the outer else-branch is live, i.e. the
inner conditions must be complementary rather than merely disjoint. That holds for a merge whose two
conditions are some `c` and `!c`, and it is *not* what the interpreter guarantees in general: flattening's
conjunction-gated nested merges are precisely the counterexample class. **Row 2 is nonetheless shipped
upstream**, so upstream is relying on some such premise — either guaranteed by construction at the sites that
build these merges, or not established at all. This record does **not** settle which: the shipped rule was
read in `simplify.rs`, not re-derived from `flatten_cfg`'s generation sites, and the throwaway clone the
experiment ran in is gone, so no re-measurement was possible while writing this section. It is captured as
follow-up work. Consequences for this record:

- **Row 3 — the half that was measured and is the only half this record would ever propose — is
  unaffected.** It reads the then-value and stands on *at-most-one* alone.
- **Row 4 is not claimed sound**, its mirrored implementation is **not** offered as reconstructable, and it
  must not go upstream until the stronger premise is established for the shapes it can match (§7, §9).

A separate trap is already pinned by an upstream regression test: the outer merge's `else_condition` must be
left alone, never replaced by the inner one (`flatten_cfg.rs:2531`,
`do_not_replace_else_condition_with_nested_if_same_then_cond` — the fix for #6875's original form). The
implementation below only ever rewrites a *value*, never a condition.

## 3. The stated blocker is stale

The TODO blames "duplicate NOT instructions having distinct ValueIds". Git archaeology says that reason
expired the day it was written:

| evidence | consequence |
|---|---|
| the TODO arrives in **#6875** (TomAFrench, `1a0a5f6123`, 2024-12-19), the PR that added rows 1–2 | the comment describes the state of the tree *that morning* |
| **#6886** (jfecher, `9108df3aa4`, **the same day**) adds `Context::not_instructions`, a `cond → Not(cond)` memo used by every `Not` flattening inserts, with the commit comment *"keeps ids unique which helps simplifications"* | the root cause the TODO names was fixed hours later, and nobody went back to the TODO |
| `simplify.rs`'s `Not` arm folds `!!v => v` (`simplify.rs:179-186`) | `Not(Not(c))` never survives as a distinct id, so the `c` vs `!!c` spelling of the pattern collapses to plain id equality |
| no open upstream issue tracks this TODO (GitHub issue search, 2026-07-27) | under the program's §6 issue-first protocol there is nothing to link a PR to |

So the sketch's harder half — CSE'ing `Not`, or canonicalizing negation — is **not needed** to enable the
merge. Plain `ValueId` equality is enough for the shapes the tree actually produces. Duplicate `Not`s do
still exist in flattened SSA (three distinct `not v0` values appear in the snapshot of the regression test
cited above), so a structural comparison is not *vacuous* — it is just never the thing standing in the way.

## 4. The implementation (written, tested, not proposed)

Both TODO sites were replaced by one binding of all four inner fields plus a `same_condition` helper that
subsumes the sketch's NOT-canonicalization half. Shape (probe instrumentation from §6 elided):

```rust
/// True when `a` and `b` are the same boolean condition, even if they are distinct `ValueId`s.
fn same_condition(dfg: &DataFlowGraph, a: ValueId, b: ValueId) -> bool {
    if a == b {
        return true;
    }
    match (dfg.get_local_or_global_instruction(a), dfg.get_local_or_global_instruction(b)) {
        (Some(Instruction::Not(a)), Some(Instruction::Not(b))) => a == b,
        _ => false,
    }
}

// …in the `Instruction::IfElse` arm, replacing the first TODO:
if let Some(Instruction::IfElse {
    then_condition: inner_then_condition,
    then_value: inner_then_value,
    else_condition: inner_else_condition,
    else_value: inner_else_value,
}) = dfg.get_local_or_global_instruction(then_value)
{
    let inner_value = if same_condition(dfg, then_condition, *inner_then_condition) {
        Some(*inner_then_value)          // row 1, unchanged behaviour
    } else if same_condition(dfg, then_condition, *inner_else_condition) {
        Some(*inner_else_value)          // row 3, the TODO
    } else {
        Option::None
    };
    if let Some(then_value) = inner_value {
        return SimplifiedToInstruction(Instruction::IfElse {
            then_condition, then_value, else_condition, else_value,
        });
    }
}
// …and the mirrored block for `else_value` (rows 2 and 4) — written at the time, but see the caveat below:
// row 4 is NOT justified by the invariant this record establishes, so that block is not offered here.
```

Three unit tests were added to `simplify.rs`'s test module, driving `Ssa::from_str_simplifying` on
array-typed merges: the row-3 shape, the row-4 shape, and a row-3 shape whose conditions are **two distinct
`not v0` instructions** (the case `same_condition` exists for). **Non-vacuity was
checked**: all three FAIL against HEAD's logic and PASS with the patch.

**Caveat on the row-4 half, added on review.** §2 shows that rows 2 and 4 need an *exactly-one* premise on the
inner merge that *at-most-one* does not supply, and the row-4 unit test pins the collapse unconditionally —
so that test encodes a rewrite this record cannot justify, and none of the evidence below (green suite,
unmoved snapshots, green fuzzer targets) speaks to it: no test in the tree exercises an outer selected branch
with **both** inner conditions false, which is the case that separates the two premises. Only the row-3
(then-value) half is offered. The row-4 arm and its test would have to be dropped, or guarded by an actual
complementary-conditions check, before anything went upstream — and since nothing fires either way (§6), the
verdict in §7 is unchanged. The full suite stays green — `cargo test -p
noirc_evaluator`: **1887 passed, 0 failed** (1884 on HEAD plus the three added here), and **no SSA/ACIR
snapshot moved**. The AST fuzzer's quick
property targets (`cargo test -p noir_ast_fuzzer_fuzz arbtest`, incl. `min_vs_full`, `acir_vs_brillig`,
`pass_vs_prev`) are green on the patched compiler — evidence of behaviour preservation, not a proof.

## 5. Where such a merge can fire at all

A structural consequence of the same function, worth recording because it bounds the whole opportunity: the
nested-merge collapse can only match when the *inner* value is still an `Instruction::IfElse`. For a
**numeric** type the `IfElse` arm ends by lowering the merge to arithmetic via
`ValueMerger::merge_numeric_values`, so a numeric merge inserted through the simplifying path is never an
`IfElse` by the time an outer merge inspects it. The rules therefore only ever fire on **array/vector-typed
merges** — the expensive kind, which is the good news — and the measurement in §6 agrees: every firing
observed came from array-merge-heavy packages.

## 6. Measurement — how often does it fire?

Method: the four collapse sites were instrumented with an env-gated counter (`NOIR_IFELSE_PROBE`) tagging
each firing by site (`then/else` and `else/else` are the new rows 3–4) and by whether plain `ValueId`
equality sufficed (`exact`) or the structural `Not` comparison was needed (`structural`). A release `nargo`
was built from the patched tree and run over every package of both corpora with `nargo info --json`.

| corpus | packages | row 1 `then/then` | row 2 `else/then` | **row 3 `then/else`** | **row 4 `else/else`** | `structural` |
|---|---:|---:|---:|---:|---:|---:|
| noir `test_programs/{benchmarks,execution_success}` | 518 | 4 | 255 | **0** | **0** | **0** |
| sparq `zk/{compose,xpath}` (copied out of the repo) | 38 | 0 | 0 | **0** | **0** | **0** |
| `noirc_evaluator` unit suite (1887 tests, hand-written SSA) | — | 2 | 0 | **0**\* | **0**\* | **0**\* |

\* excluding the three tests added by this experiment, which are the only things in the tree that fire rows 3–4.

Reading:

- **The TODO'd merge never fires.** Not once, on 556 real packages, including the four heaviest upstream
  benchmarks and every sparq circuit.
- **The NOT-canonicalization half is dead weight even for the rules that do fire**: all 259 firings matched
  by plain `ValueId` equality; the structural comparison never added a match. This confirms §3 empirically.
- The existing rows 1–2 do fire, but narrowly: 259 firings in **3** of 518 packages
  (`large_nested_array_multi_field_merge` 204, `…_u64` 51, `regression_13040` 1) — all deliberate
  nested-array-merge regression fixtures.
- Because nothing new fired, **ACIR is unchanged**. Verified rather than deduced: the whole 518-package
  corpus was recompiled with a baseline `nargo` built from unpatched HEAD, and a diff of the two
  `nargo info --json` result sets is **empty** — every function of every package has identical opcode
  counts. As a cross-check on the environment, the shared benchmarks reproduce the
  program record's §5.2 baseline exactly — `sha512_100_bytes` 13173, `semaphore_depth_10` 5699,
  `bench_eddsa_poseidon` 4147, `bench_poseidon2_hash_100` 202.

Why zero, structurally: row 3 needs source of the shape `if c { if !c { a } else { b } } else { d }` — an
inner branch that is dead by construction — surviving to the point where both merges are array-typed and
adjacent. Real programs do not contain it, and the compiler does not manufacture it: flattening gates a
nested branch by the **conjunction** `c & c2`, which is neither `c` nor `!c`.

## 7. Verdict — PARK

The bead's own instruction is *"PARK with a findings note if measured neutral"*, and the measurement is
neutral in the strongest available sense: the new code path is never taken. Landing it upstream would add a
condition helper, two rewritten match arms and three tests, whose measured effect on every program anyone
has is **nothing** — precisely the shape jfecher closed **PR #11580** for (*"doesn't have enough
optimizations to warrant the upkeep"*, program record §4.2). It also has no issue to link under the
program record's §6 issue-first protocol. **No upstream PR is proposed. The patch is not carried in this repo**; it lives only in the
throwaway clone, and §4 records enough to reconstruct **the row-3 (then-value) half** in minutes. The row-4
half is deliberately not reconstructable from this record: §2 shows it is not justified by the invariant
established here.

What would change the verdict, in order of cheapness:

1. **A program that produces the shape.** If a future circuit (or an upstream pass that rewrites conditions
   — e.g. a negated-`jmpif` canonicalization, cf. #6624 in §10.5) starts emitting nested opposite-condition
   array merges, re-run §6's probe; the row-3 half is then a one-line-behaviour change with tests ready
   (the row-4 half additionally needs §2's exactly-one premise settled first).
2. **A gate-level check**, if anyone wants one despite a zero firing count. Not run here (`bb` absent) and
   not worth a box: with no firing there is no difference to measure.
3. **A comment-accuracy fix upstream.** The TODO as written misinforms the next reader — its blocker was
   removed by #6886 on the same day (§3). Correcting it is a two-line comment change; upstream closes
   typo-scale PRs on sight (§4.2), so it should ride along with a substantive PR in this file rather than go
   up alone. **Drafted, not posted, and not recommended as a standalone.**

## 8. Reproduction (and the toolchain recipe the sibling beads are blocked on)

§10.5–10.7 all record the same blocker — the box pins rustc `1.89.0` while only `1.88.0` is installed and
`/usr/local/rustup` is read-only. The way through is that `rustup` honours a **writable `RUSTUP_HOME`**:

```bash
export RUSTUP_HOME=/home/worker/rustup                 # any writable path
rustup toolchain install 1.89.0 --profile minimal      # the trailing /usr/local/cargo/bin error is harmless
cd <noir clone>
RUSTUP_HOME=$RUSTUP_HOME RUSTUP_TOOLCHAIN=1.89.0 cargo test -p noirc_evaluator      # ~4 min build, 4 cores
RUSTUP_HOME=$RUSTUP_HOME RUSTUP_TOOLCHAIN=1.89.0 cargo build -p nargo_cli --release # ~25 min
```

`cargo test` **captures test stderr**, so an instrumentation `eprintln!` needs `-- --nocapture` (an early
run of this experiment read "0 firings" purely because of that). Corpus sweep: for each package directory,
`NOIR_IFELSE_PROBE=1 nargo info --json --silence-warnings`, aggregating stderr probe lines and the
per-function `opcodes` fields. The sparq circuits must be **copied out of the sparq checkout first** —
`nargo` writes a `target/` next to each `Nargo.toml` and would dirty the repo. `bb` was still not obtained,
so gate counts remain unavailable on this box.

## 9. Honest limits

- **The else-value collapses (rows 2 and 4) are not justified here, and row 2 is already upstream.** §2 gives
  the counterexample: an outer selected branch leaves the matched inner condition false, and with the other
  inner condition false too the nested merge is zero, not the arm the rewrite substitutes. Whether upstream's
  shipped row 2 is safe by construction at the sites that build these merges was **not** checked — that needs
  a re-read of `flatten_cfg`/`ValueMerger` and a test with both inner conditions false, neither of which this
  record has. Treat the row-2/row-4 rewrites as **unjustified pending that check**, not as validated.
- **No gate measurement** (`bb` absent). Immaterial here — nothing fires — but it means this record cannot
  be cited as gate evidence for anything else.
- **No upstream thread was read.** #6875/#6886 intent is inferred from commits, code comments and the
  regression test, not from the PR discussions; the "nobody went back to the TODO" reading is an inference
  from the tree's state at HEAD.
- **Corpus, not the world.** 556 packages is the corpus this program uses; a firing count of zero on it is
  strong evidence of rarity, not proof of impossibility. Aztec's pinned real circuits (upstream CI's
  benchmark set) were not compiled.
- The patch was reviewed by its author only; it is not maintainer-reviewed, and per `AGENTS.md`
  § *Upstream contributions* nothing here has been posted upstream.
