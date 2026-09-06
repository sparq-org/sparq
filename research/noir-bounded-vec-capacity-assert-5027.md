# `BoundedVec::push` capacity-assert slice of noir#5027 — status + pre-flip review (sq-b0vpc)

**Bead:** `sq-b0vpc` (epic `sq-uuvac`, `noir-optimization-program.md` §7 row 11, §10.3 *"`noir_stdlib/src/collections/bounded_vec.nr`; only the TomAFrench capacity-assert slice of #5027"*) ·
**Status:** **the implementation ALREADY EXISTS upstream as draft PR [noir#13314](https://github.com/noir-lang/noir/pull/13314)** (authored 2026-07-10, still `DRAFT`, unmerged). No new implementation was written for this bead, and none should be — the remaining work is the human author-review gate ([sparq#1840](https://github.com/sparq-org/sparq/issues/1840)), plus the two pre-flip findings in §3 ·
**Author:** SPARQ agent 🤖 [OPUS-5] · **Date:** 2026-07-29 ·
**Amended 2026-08-01** (SPARQ agent 🤖 [OPUS-5], issue #5062): §5 added — the
§3.1 finding named a missing test but not what it is, so §5 specifies, command-by-command,
one *prerequisite* of it: the ACIR memory-bound **mechanism** test (L1) plus the
positive control that keeps it non-vacuous. §5 does **not** specify the
`BoundedVec::push` regression guard itself — the `BoundedVec` form (§5.6 item 2)
is still unresolved and returns no verdict as shaped, so §4 item 2 stays open.
§5 is a SPECIFICATION; it has not been run, and no line of it is a measurement.
**Amended 2026-08-01** (SPARQ agent 🤖 [OPUS-5], issue #5063): §6 added — the §3.2
finding called the gate row unreproducible and stopped there. §6 quotes the table
verbatim, identifies the fixture it names (it exists, in the #5027 thread; it is **not**
in the PR), records three checks that need no toolchain — one of which finds the two
"0 delta" corpus rows **non-evidential** — and splits §4 item 1 into the provenance
answer (unblocked today) and the re-take (still blocked on `sq-i50o4`). §6 measures
nothing: this session had no `nargo` and no `bb` either.

## 0. What this record is, and what it is not

This bead was picked up cold by a fleet agent whose brief said "implement the
BoundedVec capacity-assert slice". It is already implemented. This record exists so
the next agent to read §10.3 does not re-derive a change that has been sitting
upstream since 2026-07-10.

Environment, stated up front because it bounds every claim below:

| capability | available | consequence |
|---|---|---|
| upstream PR/issue **web** pages (unauthenticated, read-only) | yes | PR state, diff and commit message are **verified**, quoted from `13314.diff` / `13314.patch` |
| upstream `master` sources (`raw.githubusercontent.com`) | yes | the line citations in §2–§3 are **verified at `master` on 2026-07-29** |
| noir checkout / `cargo` / `nargo` / `bb` | **no** | **none of the five `noir-optimization-program.md` §10.2 acceptance criteria were run in this session.** No number below is a measurement taken here |
| the issue #5027 discussion thread | **not read** — only the PR's own reference to it | TomAFrench's original framing is carried from §7 row 11 unverified |

Nothing was posted upstream from this bead.

## 1. Verified upstream state (2026-07-29)

| fact | value |
|---|---|
| PR | noir-lang/noir#13314, *"feat(stdlib): move BoundedVec push capacity check to unconstrained hint"* |
| author / state | @jeswr · `DRAFT`, unmerged (review state not checked) |
| commit | `efec45ef2e6a562b2c452bcfac84ff4fa9a9014c`, authored 2026-07-10 |
| files touched | `noir_stdlib/src/collections/bounded_vec.nr` only (+19 −1) — exactly the §10.3 target, no scope creep |
| `master` today | still carries `assert(self.len < MaxLen, "push out of bounds")` at `bounded_vec.nr:182` — the slice has **not** landed |

The change replaces the constrained assert in `push` with a call to a new
`unconstrained fn assert_push_in_bounds(len, max_len)` inside an `unsafe {}` block,
so the check still produces its error message during witness generation but emits no
ACIR constraint.

This matches the draft-first and one-optimization-per-PR halves of the §6 upstream
protocol. The §6 disclosure line is **not** verified here — the PR *body* was not
read, only the diff and the commit message. The block is on @jeswr, not on the fleet.

## 2. The safety argument, and where it actually lives

The commit message justifies the elision as:

> In constrained (ACIR) mode the ACVM already enforces array-write bounds: a write at
> `self.storage[self.len]` when `len >= MaxLen` raises
> `OpcodeResolutionError::IndexOutOfBounds`, so safety is fully preserved.

That names an **ACVM execution-time error**. An executor-side error is a property of
the honest prover's witness-generation run; on its own it does not establish that the
*constraint system* rejects an out-of-range write, which is the property that matters
once the assert is no longer constrained. §10.2 states the stake exactly: a wrongly
elided constraint under-constrains the circuit, and that is a soundness bug, not a
perf bug.

The stronger, in-tree argument the PR does not make is that upstream **already relies
on this same mechanism for every plain array index**. At `master`:

- `ssa_gen/context.rs:131-136` — `array_index_needs_explicit_oob_check(runtime, array_type)`
  returns `runtime.is_brillig() || array_type.element_size().0 != 1`.
- `ssa_gen/mod.rs:536-586` (read, and the shared `LValue::Index` path) and
  `context.rs:893-909` (write) call `codegen_access_check` **only when that
  predicate holds**.

So for ACIR with a simple element type (`element_size == 1`) the compiler emits *no*
explicit bounds constraint for `self.storage[self.len] = elem` and leans on ACIR's
built-in memory-op check — the in-code comment at `ssa_gen/mod.rs:591` says so
outright ("ACIR has built-in OOB checks"). For a composite element type the predicate
holds and `codegen_access_check` *is* emitted. Either way the array write carries a
dominating bound.

That reframes the residual risk honestly: if ACIR's built-in memory-op bound were not
binding in-proof, **every simple-element array index in noir would already be
under-constrained**, which is a far larger claim than this PR. The elision is
therefore *plausibly* dominated. It is **not certified sound here** — no acceptance
criterion was run in this session, and per `sq-qhy4` this repo does not label
unaudited ZK work sound.

## 3. Two findings a reviewer must clear before flipping the draft to ready

**3.1 The PR ships no test that goes red if the elision is wrong.**
`bounded_vec.nr:1353-1354` at `master` already has
`#[test(should_fail_with = "push out of bounds")] fn push_to_full_vector()`. With the
patch that test still **passes**, because the assert still fires during execution —
which is the whole point of routing it through the unconstrained hint. So the existing
test cannot distinguish "constrained check" from "unconstrained hint", and the PR adds
none that can.

Nor does §10.2's step-2 differential (`nargo execute --force` vs `--force-brillig`)
close that gap. Both sides are *execution* paths, and the new unconstrained assertion
makes **both** reject during honest witness generation whether or not any constraint
binds a malicious witness — so the differential stays green under exactly the
under-constrained implementation it would need to catch. Run it as what it is, an
execution-equivalence check, and not as the soundness regression guard.

A witness for the actual property has to bypass or mutate honest witness generation —
supply a witness whose out-of-range `self.storage[self.len]` write is *not* caught by
the prover-side hint — and show that the constraint system / verifier rejects it.
**No test available today establishes that**, neither in the PR nor at `master`, and
this record does not claim one exists; the flip stays blocked pending one (§4 item 2).
§5 specifies a *prerequisite* of that test — the fixture, the forge, the oracle and the
positive control for the ACIR memory-op bound the elision leans on (L1). The guard that
exercises the **patched `push` write itself** is not specified: §5.6 item 2 records why
the `BoundedVec` form returns no verdict as currently shaped, and leaves the reshaping
open.

**3.2 The measured table is not reproducible in-repo.**
The commit message carries a before/after table (ACIR opcodes and UltraHonk gates on
the SIZE=500 reproducer) plus "0 delta" on the standard benchmark corpus and on the
sparq compose corpus. Two observations, neither of which establishes the numbers as
taken:

- *Consistent, but not evidence of provenance:* "sparq compose corpus (8 packages)"
  and the four named standard benchmarks match §5.2's baseline corpus exactly — 8
  compose packages and 4 noir benchmark packages. That is a consistency check on the
  **labels** only. Matching names and package counts do not establish that the
  commands were run, that the reported values came from those runs, or what
  toolchain/commit produced them; measurement execution and provenance remain
  **unverified**.
- *Not reproducible:* the gate row is the one figure later program records say the fleet
  environment could not produce — §10.8's capability table records no `bb`, and §10.10
  records "no `nargo`, no `bb`, no build". The harness bead `sq-i50o4` that would make
  any of this re-runnable has not landed. The row is therefore **unreproducible today**,
  which is not the same as wrong: the PR predates both records, and §5.2 says gate
  baselines are taken per-PR on ephemeral workspace scripts.

§5.1 makes the backend gate count the arbiter and §4.2's PR #10159 lesson is that
opcode counts alone mislead — so this row is load-bearing and it goes upstream under a
named human author. Confirming its provenance (or re-taking it) is a precondition for
the flip, not a nicety.

## 4. Recommendation

The bead's implementation work is **done and should not be repeated**. The open
actions, in order:

1. @jeswr confirms the §3.2 gate row's provenance or re-takes it. §6 (2026-08-01,
   issue #5063) splits this into three, only one of which is blocked:
   - **1a — the provenance answer.** Fixture source, `nargo` version, `bb` version,
     `bb gates` invocation. **Unblocked today**, and a precondition of 1b: the commit
     message carries none of the four, so today *nobody* can re-take the row (§6.1–6.2).
   - **1b — the re-take.** Specified command-by-command in §6.4; blocked on `sq-i50o4`,
     and even then it is corroboration, not the upstream arbiter (§6.4, §5.1 item 3).
   - **1c — repair or drop the two "0 delta" corpus rows.** §6.3(a) establishes that no
     package in the sparq compose corpus or its dependency closure calls
     `BoundedVec::push`, so that row is entailed by the patch's shape and would read
     identically for a wrong patch. **Unblocked**, and independent of `sq-i50o4`.
2. Add a test that is red when the capacity bound is not enforced at constraint level
   (§3.1) — one that bypasses or mutates honest witness generation and demonstrates
   verifier/constraint-system rejection of an out-of-range index, **not** an
   execution-path differential. Until such a test exists the PR carries no regression
   guard for the property the elision relies on, and stays blocked on it.
   §5 specifies only its **prerequisite** — the L1 mechanism test (fixture, forge,
   oracle, positive control), which goes red if the compiler stops emitting the ACIR
   memory-op bound, but **not** if a later change to `BoundedVec::push` moves or drops
   the write that bound is supposed to dominate. The `push`-path guard itself stays
   unspecified (§5.6 item 2); both are unimplemented and unrun, and blocked on a box
   with `nargo` + `bb` (`sq-i50o4`).
3. Upgrade the commit/PR safety justification from the ACVM execution-time error to
   the `array_index_needs_explicit_oob_check` contract (§2), which is the citable
   in-tree reason and is the argument a noir maintainer will want to see.
4. Then flip draft → ready per §6. No agent arms anything; upstream merge is the only "arm".

## 5. The ACIR memory-bound mechanism test, specified — a prerequisite of the guard, not the guard itself (2026-08-01, issue #5062)

> 🤖 **SPARQ agent** [OPUS-5]. §3.1/§4 item 2 established that the guard is missing
> and that the §10.2 step-2 differential is not it. They did not say what it *is*.
> This section specifies as much of it as is settled: the **L1 mechanism test** that
> pins the ACIR memory-op bound the elision leans on, together with its forge, its
> oracle and its positive control. It does **not** cover the patched `BoundedVec::push`
> write — see §5.6 item 2 for what has to be resolved before an L2 fixture returns any
> verdict at all. **Nothing here has been executed** — this session had no noir
> checkout, no `nargo` and no `bb` (`command -v nargo bb` empty), so §5 contains no
> measurement and predicts no outcome. What it does contain is verified: every
> upstream fact below carries the file it was read from, re-fetched at `master` on
> 2026-08-01.

### 5.1 The obligation, stated as a rejection

> **G.** For the patched `push`, there exists no assignment to the circuit's witnesses
> that (a) is accepted by the constraint system, and (b) writes at index `len` with
> `len >= MaxLen`.

`G` is what the guard must eventually pin. What §5.3 onward actually specifies is one
step short of it: L1 tests `G`'s **premise** — that the ACIR memory op binds an index
at all — on a stand-in fixture, not `G` on `push`. Closing the remaining step is §5.6
item 2, which is unresolved.

`G` is a property of the *constraint system*, quantified over *all* witnesses. Every
instrument currently on the table quantifies over the ACVM's *one honest* witness
instead, which is why none of them can see it:

| instrument | what it quantifies over | verdict |
|---|---|---|
| `push_to_full_vector` (`bounded_vec.nr:1353-1354`) | the honest execution | blind — the unconstrained hint still fires, so it passes either way (§3.1) |
| §10.2 step-2 `nargo execute --force` vs `--force-brillig` | two honest executions | blind — both are execution paths (§3.1) |
| any `#[test]` in `bounded_vec.nr` | the honest execution | blind by construction: `nargo test` runs the ACVM, and Noir has no in-language way to name a witness the ACVM would not have produced |

So the guard cannot live in `bounded_vec.nr`. It is necessarily a *harness* test:
compile → solve honestly → **mutate the witness outside the ACVM** → ask the
constraint system.

### 5.2 Why the mutation has to happen outside the ACVM

The ACVM is the witness solver *and* the thing that raises `IndexOutOfBounds`. Every
route that asks it for a capacity-violating witness — a private `len` input, an
`unconstrained` hint returning `MaxLen`, `--force-brillig` — is refused by the same
memory-op solver whose in-proof counterpart is the thing under test. The witness must
therefore be built by editing the artifact `nargo` already wrote.

That is a supported operation on a published API, not byte surgery
(`acvm-repo/acir/src/native_types/witness_stack.rs`, `witness_map.rs`, crate `acir`
`1.0.0-beta.26`):

```rust
// forge.rs — the whole mutation. `acir` is the only new dependency.
use acir::native_types::{Witness, WitnessStack};
use acir::FieldElement;

/// Rewrite every witness currently holding `from` to `to`. Returns how many were hit.
fn forge(path_in: &Path, path_out: &Path, from: u64, to: u64) -> std::io::Result<usize> {
    let mut stack = WitnessStack::<FieldElement>::deserialize(&std::fs::read(path_in)?).unwrap();
    let mut item = stack.pop().expect("one stack item");
    let hits: Vec<Witness> = item.witness.clone().into_iter()
        .filter(|(_, v)| *v == FieldElement::from(from))
        .map(|(w, _)| w).collect();
    for w in &hits { item.witness.insert(*w, FieldElement::from(to)); }
    let out = WitnessStack::from(item.witness);   // single-item stack: `impl From<WitnessMap>`
    std::fs::write(path_out, out.serialize().unwrap())?;
    Ok(hits.len())
}
```

`WitnessStack::{deserialize, serialize, pop, push}`, `WitnessMap::{insert, get}` and
the `From<WitnessMap>` used above are all `pub` at `master`; the sketch assumes a
single-item stack (no IVC folding), which the fixture guarantees.

### 5.3 The fixture, and why it is shaped this way

The fixture is *not* `BoundedVec` (see §5.6 for why, and for what the `BoundedVec`
form additionally needs). It is the mechanism the elision actually leans on — ACIR's
built-in memory-op bound for a simple element type, per
`array_index_needs_explicit_oob_check(runtime, array_type) = runtime.is_brillig() ||
array_type.element_size().0 != 1` (`ssa_gen/context.rs:131-136`, re-verified at
`master` 2026-08-01; consumers at `context.rs:893` and `ssa_gen/mod.rs`). For ACIR
with `element_size == 1` that predicate is false, so **no explicit bounds constraint
is emitted** and the ACIR memory op is the only thing standing between a forged index
and an accepted witness. That is exactly `G`'s premise.

```noir
// L1 — the mechanism fixture. MaxLen = 4.
fn main(idx: u32, elem: Field) -> pub Field {
    let mut storage: [Field; 4] = [0, 0, 0, 0];
    storage[idx] = elem;   // the ONLY consumer of `idx`
    storage[0]             // reads slot 0 => the array cannot be DCE'd
}
```

Honest inputs: `idx = 3`, `elem = 7`. Three properties are load-bearing and each is a
deliberate choice, not an accident:

- **`idx` feeds nothing but the write index.** A forged `idx` therefore leaves every
  other constraint satisfied, so the *only* constraint that can reject is the bound.
  Had `idx` also fed, say, a `len += 1`, mutating it would strand the dependent
  witness and the circuit would reject for an unrelated reason — a false red that
  reads exactly like a pass.
- **The return reads slot 0, not `idx`.** The public output is `0` under the honest,
  the in-range-forged and the out-of-range-forged witness alike, so the public-input
  check cannot be what rejects.
- **`3` is *intended* to be unique in the witness map.** The zero-filled array and
  `elem = 7` are chosen so no unrelated witness holds `3` — but what intermediate
  witnesses ACIR-gen introduces was **not verified here**, which is precisely why the
  forge returns its hit count and the harness asserts it. A value collision would
  silently mutate an unrelated witness and, again, produce a false red; an unasserted
  hit count would hide it.

`idx: u32` costs a 32-bit range check; `3`, `4` and `2` all satisfy it, so it is
never what rejects either.

### 5.4 The oracle

`bb` grew a general circuit-satisfaction subcommand — verified in
`barretenberg/cpp/src/barretenberg/bb/cli.cpp` at `master` 2026-08-01, which registers
`check` ("*a debugging tool to quickly check whether a witness satisfies a circuit …
constructs the execution trace and iterates through it row by row, applying the
polynomial relations defining the gate types*") alongside `prove`, `verify`, `gates`
and `write_vk`. Flags, from the same file: `--scheme,-s` (default `ultra_honk`),
`--bytecode_path,-b`, `--witness_path,-w`.

```sh
nargo compile                                    # target/<pkg>.json
nargo execute honest                             # target/honest.gz  (idx = 3)
# forge.rs: honest.gz -> forged_oob.gz (3 -> 4) and forged_ok.gz (3 -> 2)

bb check  -s ultra_honk -b target/<pkg>.json -w target/forged_oob.gz     # fast pre-filter
bb prove  -b target/<pkg>.json -w target/forged_oob.gz --write_vk -o out # authoritative
bb verify -k out/vk -i out/public_inputs -p out/proof                    # exit code IS the verdict
```

**`prove` + `verify` is the authoritative oracle, not `check`.** The bound at stake
lives in the RAM-consistency argument, whose relations are evaluated against grand
products the prover computes; whether `bb check`'s row-by-row pass covers that
argument was **not verified here**, so `check` is a cheap pre-filter only and a green
`check` settles nothing. The repo already owns the authoritative half —
`sparq_zk_compose::driver` shells `bb prove`/`bb verify` and surfaces rejection as
`Ok(false)` (`crates/sparq-zk-compose/src/driver.rs`) — so only the forge is new.

### 5.5 The verdict matrix — and the control that stops a vacuous pass

Three runs, and **all three verdicts are required**; two of them are the guard and the
third is what makes the guard mean anything.

| # | witness | required `bb verify` | reads as |
|---|---|---|---|
| 1 | honest (`idx = 3`) | **accept** | the fixture proves at all |
| 2 | forged in-range (`3 -> 2`) | **accept** | *the positive control*: mutating this witness set is self-consistent |
| 3 | forged out-of-range (`3 -> 4`) | **reject** | the guard: the constraint system binds the index |

Run 2 is the whole reason this design is worth landing. Runs 1+3 alone are satisfied
by a harness that mutates the witness into incoherence — every forged witness
rejects, the test is green, and it would stay green under a circuit with no bound at
all. Run 2 is the mutation check: same witness set, same edit machinery, different
value, and it must still verify. **If run 2 rejects, the harness must report VACUOUS
and fail** rather than reporting a pass on the strength of run 3.

What each outcome licenses, stated narrowly:

- **Runs 1,2 accept and 3 rejects** — the constraint system rejects *this* witness
  class on *this* fixture. That is a regression guard for the **compiler mechanism**:
  it goes red if a future compiler change stops emitting the bound. It does **not** go
  red if `BoundedVec::push` itself later drops, moves or reshapes the write that bound
  is supposed to dominate — pinning that is L2's job (§5.6 item 2), still unspecified.
  Nor is it a soundness proof of `G` (one witness is not a quantifier); per `sq-qhy4`
  this repo does not label it sound.
- **Run 3 accepts** — a witness violating the capacity bound is accepted by the
  constraint system. That is the under-constraining outcome §10.2 calls a soundness
  bug, and it blocks the flip outright.
- **Run 2 rejects** — no verdict. The harness is measuring its own mutation, not the
  circuit. Fix the fixture, do not report a result.

### 5.6 The second vector, and the `BoundedVec` form

Two extensions, in priority order:

1. **The composite-element control.** Re-run L1 with `[(Field, Field); 4]`
   (`element_size == 2`), where `array_index_needs_explicit_oob_check` *is* true and
   an explicit check is codegen'd (`context.rs:893-905`). Rejection on both variants
   distinguishes "the ACIR memory-op bound binds" from "only the explicit check
   binds" — i.e. it tests §2's argument, which is the one a noir maintainer will be
   asked to accept.
2. **The `BoundedVec` form itself (L2).** `BoundedVec::from_parts_unchecked` asserts
   `len <= MaxLen` (`bounded_vec.nr:610-613`) — note `<=`, so a forged `len == MaxLen`
   survives it and only `push`'s write can catch it, which is what makes L2
   expressible at all. But L2 fails §5.3's first property twice over: `push` does
   `self.len += 1`, and `from_parts_unchecked`'s comparison carries Brillig-hinted
   witnesses; mutating `len` strands both, so L2 lands in the run-2-rejects cell and
   returns *no verdict* unless the fixture is first reshaped so `len` reaches nothing
   but the write. **Do not ship L2 before its run 2 is green** — an L2 that reports
   run 3's rejection while run 2 also rejects is precisely the false guard §3.1
   warns about, one layer up. That reshaping is **not designed here**, so this record
   specifies no test that reaches the patched `push` write; whatever shape it takes, it
   owes §5.5's full honest / in-range-forged / out-of-range-forged matrix on that real
   path. Until it exists, §4 item 2 stays open and §5 covers the mechanism only.

### 5.7 Placement, cost, blockers

- **Home.** Not upstream `bounded_vec.nr` (§5.1). The practical home is this repo's
  existing noir-toolchain lane — a fixture package beside
  `crates/sparq-zk/tests/fixtures/noir_poseidon2/` (same "run it if `nargo` is on
  PATH, else skip loudly" convention as `tests/poseidon2_noir_cross.rs`), with the
  `bb` half reusing `sparq_zk_compose::driver`. Upstream gets the *result* and the
  fixture, quoted in the PR thread under a named human author per §6.
- **Cost.** One new dependency, `acir` (§5.2), which pulls `acir_field` / `brillig` /
  `rmp-serde` / `flate2`. It is dev-only and confined to the opt-in zk lane that
  nothing in the workspace depends on, but it is a supply-chain decision
  (`deny.toml`, vet, SBOM) and belongs to a reviewer, not to this record.
- **Blocked on.** A box with `nargo` + `bb` — i.e. `sq-i50o4`, the same bottleneck
  §3.2, §10.8 and §10.10 record. Until then §5 stays a specification, and §4's
  ordering is unchanged: the flip is still blocked on items 1–3.

## 6. The gate row — what is decidable today, and the re-take specified (2026-08-01, issue #5063)

> 🤖 **SPARQ agent** [OPUS-5]. §3.2 established that the commit message's gate row
> cannot be reproduced in the fleet environment and left it there. This section does
> the part that does **not** need a toolchain: it quotes the table verbatim (§6.1),
> identifies the fixture it names (§6.2), records three checks that need only text and
> this repo (§6.3), and specifies both routes out — the provenance answer, which is
> unblocked, and the re-take, which is not (§6.4). **Nothing here is a measurement.**
> `command -v nargo bb` is empty in this session too, so no number below was taken
> here; every number quoted is transcribed from the upstream commit message or
> computed from numbers already in it. The upstream facts were re-fetched on
> 2026-08-01 from `13314.patch` and the #5027 issue page.
>
> **Reference convention** (this record grew a §5 of its own on 2026-08-01, which
> collides with the program record's numbering): below, `§5.1`, `§5.2`, `§6 item 5`
> and `§10.x` are `noir-optimization-program.md`, as they already are in §3.2;
> references to *this* record are written `§6.3`, `§4 item 1`, `§5.6 item 2`.

### 6.1 The claim, quoted — and the four things the commit does not carry

Verbatim from `13314.patch`, commit `efec45ef` (§1), so this record no longer
paraphrases the thing it is auditing:

> Measured on the noir#5027 reproducer (SIZE=500 conditional-push loop):
>
> | Metric        | Before | After  | Delta   |
> |---------------|--------|--------|---------|
> | ACIR opcodes  |  9 479 |  7 483 | −21.1 % |
> | UltraHonk gates | 22 312 | 19 445 | −12.9 % |
>
> Standard benchmark corpus (sha512, semaphore, poseidon2, eddsa): 0 delta.
> sparq compose corpus (8 packages): 0 delta.
> noir_stdlib test suite: 409/409 passed.

The message names **no fixture file, no `nargo` version, no `bb` version and no
command**. §5.1 makes `bb`/`nargo` version-coupled (*"use `bbup`"*) and makes the
`circuit_size` field of `bb gates` the arbiter, so a gate count without its `bb`
version is not re-takeable *by anyone*, on any box — the block is not only
`sq-i50o4`. The commit's own author line is `Ubuntu
<ubuntu@ip-172-31-10-78.eu-west-2.compute.internal>`: a default identity on an
ephemeral EC2 host, consistent with §5.2's *"per-PR on ephemeral workspace scripts"*
story, and equally consistent with the artefacts being unrecoverable — the host is
gone and the commit metadata carries no human attestation of the run either. Note
also that the two corpus rows do not say **which metric** is 0-delta (opcodes,
gates, or both); §6.3(a) makes that question moot for the sparq half, but it is part
of the provenance ask for the noir half.

### 6.2 The fixture exists upstream — and it is not in the PR

"The noir#5027 reproducer (SIZE=500 conditional-push loop)" is *not* the issue's
top-post program: that one is a plain array write loop (`fields[indices[i]] = i as
Field`) with no `BoundedVec` in it, and the patch cannot move its counts at all. The
phrase resolves instead to sirasistant's comment in the same thread (last edited
2024-05-28), read on the #5027 page on 2026-08-01:

```noir
global SIZE = 500;

fn main(fields: [Field; SIZE], to_keep: [bool; SIZE]) -> pub [Field; SIZE] {
    let mut bounded_vec: BoundedVec<Field, SIZE> = BoundedVec::new();

    for i in 0..SIZE {
        if to_keep[i] {
            bounded_vec.push(fields[i]);
        }
    }

    assert_eq(bounded_vec.len(), 0);
    bounded_vec.storage
}
```

So the fixture is identifiable, which is a point in the row's favour. Three
consequences, all of which a re-take has to respect:

- **The PR ships it nowhere.** The diff is `bounded_vec.nr` only, +19 −1 (§1). That is
  a deviation from §6 item 5 of the program record, which asks for the focused
  benchmark fixture under `test_programs/benchmarks/`, and it is *why* the row is
  unreproducible independently of `sq-i50o4`: with no pinned fixture there is nothing
  to re-run even on a box that has `bb`. Pinning it is the cheapest half of the fix.
- **It is compile-only.** `assert_eq(bounded_vec.len(), 0)` contradicts the pushes, so
  the program cannot be *executed* with any witness that exercises `push`. `nargo
  compile` + `bb gates` is fine (neither needs a witness); no execution-based
  cross-check of the row is available, and §10.2's step-2 differential cannot be run on
  this fixture at all.
- **The thread's own number for this program is 1.6 M gates**, against the table's
  22 312 "Before" — a ~72× gap. Two years of upstream work on exactly this issue may
  account for all of it (that is the issue's whole subject), so this is **not** a
  discrepancy claim; it is a statement that the "Before" cell is not comparable to any
  published figure and stands or falls on its own provenance.

### 6.3 Three checks that need no `nargo` and no `bb`

All three were run in this session against this repo at `HEAD` and the pinned upstream
tags; they are text and corpus checks, not measurements.

**(a) The "0 delta" corpus rows are non-evidential — for the sparq half, provably.**
The patch changes only `BoundedVec::push`. No package in the sparq compose corpus can
call it: `grep -rn "BoundedVec\|\.push(" zk --include=*.nr` over all **55** `.nr` files
under `zk/` returns **0 hits**, and the corpus's entire dependency closure returns 0
too — all eight §5.2 compose packages depend on `sparq_zk_compose_core` alone, whose
only external deps are `poseidon` `v0.3.0` (9 `.nr` files) and `sparq_ieee754`
`v0.11.0`, both fetched at their pinned tags and grepped for the same two tokens: 0
hits each. An uncalled generic stdlib method is never monomorphised into ACIR, so
**"sparq compose corpus (8 packages): 0 delta" is entailed by the shape of the patch
and would have read identically for a broken one.** It is a true row carrying no
information: it is not a regression check, and it should not go upstream presented as
one (§4 item 1c). The four noir benchmark `main.nr`s were also fetched at `master` and
show no direct `BoundedVec` use, but their transitive deps (`sha512`, `ec`, `edwards`,
`poseidon`) were **not** audited — so that half is *suspected* vacuous, not established.

**(b) The gate row's percentage does not follow from its own two cells.**
22 312 → 19 445 is a delta of 2 867, i.e. 12.8496 %, which is **−12.8 %** at one
decimal by rounding or truncation alike — not the −12.9 % printed. The opcode row is
correctly rounded (1 996 / 9 479 = 21.057 % → −21.1 %). This impugns no count; what it
shows is that at least one cell was entered by hand rather than emitted by the script
that produced the other, which is precisely the question §3.2 asks.

**(c) Scale sanity, stated as questions a one-line answer disposes of.** The opcode
delta is 1 996 = **4 × 499** over a 500-iteration loop — ~4 ACIR opcodes per elided
`assert(len < MaxLen)`, but exactly one iteration short of 4 × 500. And the table's
gates/opcodes ratio is ≈ 2.35× before and ≈ 2.60× after, *below* the 5–50× divergence
band §5.1 records for this metric pair. Neither is an error and neither is evidence of
one; both are unexplained details that a provenance answer settles for free.

Recorded so no later reader is startled by it, and claimed as nothing more: the
"Before" gate cell, 22 312, is adjacent by one to §5.2's `scan_k2_n64_r8` **ACIR
opcode** baseline, 22 313 — different metric, different program, and no significance is
asserted. Relatedly, the commit's corpus is exactly §5.2 minus its three probe bins,
which §5.2 says were written for that assessment and *kept out of the sparq repo* —
which is consistent with §3.2's label-match observation and, like it, establishes
nothing about execution.

### 6.4 The two routes, in cost order

**Route A — confirm provenance (unblocked today; does not need `sq-i50o4`).** @jeswr
states, in the PR thread or an amended commit message, four things: (1) the fixture
source, verbatim or as a committed package; (2) the `nargo` version/commit; (3) the
`bb` version, per §5.1's `bbup` coupling; (4) the `bb gates` invocation and the field
read (`circuit_size`). This is a precondition of Route B as much as an alternative to
it — without (1)–(3) a re-take is not comparing like with like.

**Route B — re-take it (blocked on `sq-i50o4`).** Same box, same two binaries, both
halves measured in one sitting; the *diff* is the claim, but each half records its own
absolute numbers:

```sh
# 0. pin and record. Both versions go in the PR body; bb is installed via bbup (§5.1).
git -C noir rev-parse HEAD && nargo --version && bb --version

# 1. the fixture of §6.2 as its own package (bin, no deps), e.g. bounded_vec_push_500/.
#    Compile-only: do NOT nargo execute — assert_eq(len, 0) contradicts the pushes.

# 2. BEFORE, at the recorded master commit
nargo compile
nargo info --json                                   # ACIR opcode row
bb gates -s ultra_honk -b target/<pkg>.json         # read `circuit_size` — the arbiter

# 3. AFTER: apply 13314.patch to the same checkout, rebuild nargo, repeat step 2
#    with the same bb binary.
```

Two honesty constraints on the result. First, per §5.2 and §5.1 item 3 a fleet-box
`bb gates` number is **non-canonical**: upstream's arbiter is the
`noir-lang/noir-gates-diff` sticky comment, which fork PRs do not get, so a maintainer
(or @jeswr's fork CI) must surface it — a green re-take is corroboration, not the
upstream verdict. Second, the corpus rows should not simply be re-run: per §6.3(a) the
honest repair is to *replace* the sparq compose row with packages that actually call
`push`, or to drop it and say so. §5.2's baseline corpus contains no such package
today, which is why the row was vacuous in the first place.

### 6.5 What this changes

§4 item 1 is now 1a/1b/1c (see there): the provenance answer and the corpus-row repair
are **unblocked and cheap**, and only the re-take waits on `sq-i50o4`. §3.2's verdict
is unchanged and is not weakened by any of the above — the row remains unreproducible,
which remains not the same as wrong. What §6 adds is that two of the three things
blocking the flip on this axis were never blocked at all.

Companion records: `noir-optimization-program.md` (§7 row 11, §10.2 acceptance
protocol, §10.3 fleet spec, §10.11 status), `noir-optimization-new-opportunities.md`
(the `sq-i50o4` bottleneck).
