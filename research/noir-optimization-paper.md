# Optimising the Noir compiler — paper skeleton (sq-1j5ow)

<!-- [OPUS-4.8] sq-1j5ow: kickoff skeleton for a measured, upstreamed paper on
     Noir compiler optimisation. DESIGN-FOR-REVIEW / DOC-ONLY — no paper `.typ`
     source is written here and no upstream PR is opened from this bead; the
     phased plan in §5 enumerates each future bead. Grounded in the sq-uuvac
     estate (research/noir-optimization-program.md) + the four open fresh drafts
     noir-lang/noir#13263-#13266 + the HEAD compiler source read in
     ~/noir-optim-workspace/noir. -->

> 🤖 **SPARQ agent** — paper skeleton produced for bead `sq-1j5ow` (child work of
> the Noir upstream optimization program, epic `sq-uuvac`). Model: Opus 4.8 (1M
> context). This is a **design-for-review** record for the maintainer (@jeswr):
> it plans a paper, it does not write one, and it opens no upstream PR.

## 0. Brief correction (read first — honesty)

The activating brief describes "a prior program of fresh SSA-optimization
re-derivations for Noir … draft upstream PRs #13263-#13266" and "3 SSA
re-derivations". Read against the actual PR estate (checked on
[noir-lang/noir](https://github.com/noir-lang/noir) 2026-07-06), that framing is
close but imprecise, and the paper must start from the precise picture:

- **jeswr's three *originating* theme PRs** — [#12780](https://github.com/noir-lang/noir/pull/12780),
  [#12781](https://github.com/noir-lang/noir/pull/12781),
  [#12927](https://github.com/noir-lang/noir/pull/12927) — are @jeswr's own,
  developed with a weaker model while building the IEEE-754 library. All three
  are still **open** (12780/12781 draft, 12927 non-draft; no human maintainer
  review posted, only a Copilot review on 12927).
- **The program's four *fresh* draft PRs** — #13263-#13266 — were opened **on
  jeswr's fork by SPARQ agents** and are all **open drafts awaiting @jeswr's own
  author review** (he alone flips draft → ready). Of these four, only **#13263**
  is a *fresh re-derivation* of a jeswr theme (unsigned div/mod by an oversized
  constant, cf. #12781). **#13264 / #13265 / #13266 are net-new optimisations**
  found by the whole-compiler survey (`sq-rbfga`), sourced from an open issue, two
  in-code TODOs, and another open issue respectively.

So the estate is **3 originating themes + (1 re-derivation + 3 net-new) fresh
drafts = 7 PRs**, not "3 re-derivations". None of the seven is merged; the four
fresh drafts have **measured** artifacts, the three originating ones carry
jeswr's own measurements. The paper's empirical honesty turns on this
distinction, so §1 records each PR's measurement status explicitly.

One further correction the paper must carry: these are **circuit-size
(proving-cost) optimisations**. They do **not** touch, and do **not** establish,
any zero-knowledge privacy or soundness property — the paper makes no ZK-soundness
claim, and the Noir soundness question is entirely orthogonal to it.

## 1. Inventory of the existing optimization estate (measurement status)

Cost model reminder (Noir → ACIR → Barretenberg UltraHonk): the proving cost is
the **backend gate count** (`bb gates -s ultra_honk`, `circuit_size` field). ACIR
opcode counts (`nargo info`) are a cheap iteration proxy that can diverge from
gates by a large factor — a win must be confirmed at the gate level (the sparq
`noir-optimisation` skill and upstream PR #10159 both document this). Every
number below is stated with its source and metric.

### 1.1 jeswr's three originating theme PRs (prior work the program builds on)

| PR | Theme | Measurement status |
|---|---|---|
| [#12780](https://github.com/noir-lang/noir/pull/12780) | Track unsigned value ranges from `range_check` / equality constraints; propagate through lossless casts/`not`/selected arithmetic | Designed + unit-tested (`live_bit_width_*` fixtures, `cargo test -p noirc_evaluator`); **no standalone backend-gate delta** cited — a range-analysis substrate, not a self-contained win |
| [#12781](https://github.com/noir-lang/noir/pull/12781) | Simplify unsigned `lhs / C → 0`, `lhs % C → lhs` when `max(lhs) < C` (stacked on #12780) | **Measured** (jeswr): `bb gates` **2953 → 88** on `live_bit_width_large_constant_div_mod`; `live_bit_width_ranges` 3753 → 3720 — a focused-fixture number |
| [#12927](https://github.com/noir-lang/noir/pull/12927) | Full SSA value-range analysis (unsigned + signed two's-complement); drop redundant range checks, narrow `Lt`, checked→unchecked | **Measured** (jeswr): ACIR opcodes per benchmark (`live_bit_width_ranges` 91 → 63, −31%) + a compile-time table; standard circuits verified unchanged |

### 1.2 The program's four fresh draft PRs (the paper's empirical backbone)

All four measured on this work box with `bb 5.0.0-nightly.20260522`,
`bb gates -s ultra_honk`, ACIR opcodes via `nargo info --force`, against a corpus
of the noir `test_programs/benchmarks` set plus real sparq ZK circuits. The
deterministic gate/opcode counts are reproducible under the pinned toolchain; the
compile-time/memory figures in the PR bodies are work-box (indicative) numbers.

| PR | Optimisation (source) | SSA site | Measured result (fixture) | Corpus / soundness status |
|---|---|---|---|---|
| [#13263](https://github.com/noir-lang/noir/pull/13263) | Unsigned div/mod by a constant exceeding the dividend's max → `0` / `lhs` (fresh re-derivation of theme c, bound source = `get_value_max_num_bits` + a `Truncate` arm) | `ssa/ir/dfg/simplify/binary.rs` | 54 → 28 ACIR opcodes; 3843 → 3725 gates | noir benches + 13 sparq circuits + 3 xpath probes unchanged; 11 new SSA tests; unsigned-only, strict `C > 2^max−1`, div/mod-by-zero preserved |
| [#13264](https://github.com/noir-lang/noir/pull/13264) | Remove `truncate` of a value bounded by an AND-mask with a constant (issue #8628) | `ssa/opt/remove_truncate_after_range_check.rs` | 28 → 16 ACIR; `circuit_size` 4112 → 4112 (pinned by the AND-gadget lookup floor); Σ attributed gates 2873 → 2858 | benches + 11-program corpus unchanged; bound = mask bit-length (highest set bit), sound for any numeric representation; dominance discipline reused |
| [#13265](https://github.com/noir-lang/noir/pull/13265) | Single-limb `to_bits`/`to_radix` → range constraint (two in-code TODOs in `simplify_call`) | `ssa/ir/dfg/simplify/call.rs` | 28 → 15 ACIR; `circuit_size` 89 → 80; Σ attributed 73 → 64 | benches + 8-program corpus unchanged; range-check inserted **before** the cast so `flatten_cfg` predication (#8617) + the narrowing-cast validator both compose; predicated adversarial case tested |
| [#13266](https://github.com/noir-lang/noir/pull/13266) | Remove range checks / unsigned `lt`-against-constant implied by a dominating bound (new `remove_redundant_range_checks` pass, issue #9463) | new `ssa/opt/` pass, after `remove_truncate_after_range_check` | `vector_dynamic_index` 733 → 616 ACIR (−16.0%), `circuit_size` 5047 → 4772 (−5.4%); new fixture 41 → 33 ACIR | across 555 programs: 3 improve ACIR, 3 improve Brillig, **none regress**; external 39-program ZK corpus unchanged; `range_check`-elision restricted to `range_check`-derived facts (keeps cast justification visible); signed non-firing |

**Reading of the estate.** The strongest *reproducible* gate-level wins today are
those of PR #13263 (−118 UltraHonk gates on its fixture) and PR #13266 (−275
UltraHonk gates on `vector_dynamic_index`, −5.4%). PRs #13264 and #13265 are
honestly small at the backend level on their fixtures — the AND-gadget /
range-table floors dominate a tiny circuit — and their bodies say so; their win
is in ACIR opcodes, witnesses,
and dropped Brillig hint work, which the paper must present as such rather than
as a headline gate cut. This candour (a measured "the backend floor eats it on
small circuits") is itself part of the paper's methodological point.

## 2. Pipeline position + gap analysis

### 2.1 Where the seven PRs sit

The SSA primary pass list is `noirc_evaluator/src/ssa/mod.rs::primary_passes`
(HEAD `0df14918`, ~85 pass invocations). Every one of the seven PRs lives in the
**value-range / bit-width / instruction-simplify neighbourhood**: the
insertion-time `simplify/` rules (#13263, #13265) and the late-cleanup band at
`mod.rs:395-404` — `remove_truncate_after_range_check` (#13264), the new
`remove_redundant_range_checks` slotted right after it (#13266), and the existing
`checked_to_unchecked` that jeswr's #12780/#12927 feed. This is a single, narrow
region of a large pipeline. The paper's "further optimisations" mandate is
therefore a **gap analysis over the passes the program has not yet examined at
implementation level**.

### 2.2 Passes / stages still un-examined and plausibly optimisable

Grouped by pipeline stage, with the concrete source site and the pre-existing
upstream signal. "Un-examined" = the program has read the pass exists (survey §3)
but has not derived a measured optimisation in it. Every candidate here is a
*hypothesis to be measured*, not a claimed win.

> **P3 OUTCOME (2026-07-29, `sq-mtolx`).** Every row of this table has since been
> examined at code level against noir `8f33502e`; five are **refuted** and one is
> factually **corrected**. Verdicts are carried inline below; the full record —
> including the structural reason most of them are null, and the surviving
> candidate register — is `research/noir-optimization-new-opportunities.md`.
> Nothing was measured: the spawn gate ("reproduces a `bb`-gates win via the P1
> harness") is unmet because P1 has not landed and `bb` is unavailable, so **zero
> beads were spawned**.

| Stage | Un-examined site (file · pass) | Pre-existing signal | Why plausible |
|---|---|---|---|
| Loops | `ssa/opt/loop_invariant.rs` (LICM, ~4.5k LOC), `mod.rs:284` | issues #10439 / #10438 (LICM `ArrayGet` hoisting too pessimistic) | Loop bodies dominate loop-heavy circuits; hoisting one bounds check out of an N-iteration loop scales — **REFUTED (P3): four genuine hoisting pessimisms found, all absorbed by post-unroll CSE, so the payoff is SSA size, not gates.** One candidate survives, in LICM's *constraint-strength-reduction* half |
| Loops | `ssa/opt/unrolling.rs` (~5.2k LOC), `mod.rs:292,361` | #6631 (loop unswitching), #6629 (induction-var elimination / strength reduction, exact rewrite specified) | Unrolling already runs; unswitching + IV strength reduction are classic, upstream-requested — **REFUTED (P3): both moot under mandatory full unrolling; unswitching's risk direction is a gate *regression*** |
| Array/memory | ~~`ssa/opt/flatten_cfg/value_merger.rs::try_merge_only_changed_indices`~~ → `ssa/ir/dfg/simplify/value_merger.rs`, `mod.rs:315` | ~~#5501~~ → **#8145** (push `ArrayGet` through `IfElse`), precedent PR #11512 merged | **CORRECTED (P3): the named function was removed upstream by PR #8142, the file moved to `ssa/ir/dfg/simplify/value_merger.rs`, the live tracker is #8145, and the #5501 ask is already implemented in `flatten_cfg::try_optimize_array_set_merge`.** The "~2N constraints for one read" figure holds only for dynamic-index or escaping merged arrays; a narrower residual candidate survives |
| Array/memory | `ssa/opt/array_set_window_optimization.rs` (~951 LOC, `mod.rs:320`), `mutable_array_set.rs` (`mod.rs:417`) | none direct | Post-flatten array-write windows; unexplored — **EXAMINED (P3): no candidate; both conservatisms are a documented compile-time guard and a regression-tested liveness analysis** |
| Array/memory | `ssa/opt/mem2reg.rs` (~1.7k LOC, many invocations) | inline ordering comments (`mod.rs:212-214`) | Memory SSA promotion; cross-block promotion after flattening — **REFUTED (P3): not proof-cost-relevant (reference memory ops never reach ACIR-gen), and the hypothesised post-flatten cross-block run already exists at `mod.rs:325`** |
| Conditionals | `ssa/opt/basic_conditional.rs` (~782 LOC, `flatten_basic_conditionals`, `mod.rs:352`) | none direct | Unconstrained-only conditional simplify; unexplored — **CLOSED (P3): Brillig-only by an early return, so it emits no gates** |
| Signed lowering | `ssa/opt/expand_signed_math.rs` (`mod.rs:310`), `expand_signed_checks.rs` (`mod.rs:210`), `check_u128_mul_overflow.rs` (`mod.rs:374`) | none direct | Signed div/lt and u128 overflow lower to euclidean divisions — the expensive primitive — **EXAMINED (P3): a per-operation euclidean-division cost table was produced; one candidate survives (signed `>>` is lowered through the generic signed-division expansion), and two apparently-wasteful constructs are load-bearing with in-code derivations** |
| ACIR-gen | comparison lowering `acir/acir_context/mod.rs:1163-1228` (sign-bit via euclidean division; quotient is 0/1) | **landmine** PR #10159 (witness-sharing regressed gates) | Every `<`/`>=` pays quotient+remainder range checks; measure-first, gates-not-opcodes — **ALREADY NULL-RESULTED (`sq-jfkwk`, program §10.7) and RE-CONFIRMED at pin `8f33502e` by P3.** The adjacent 128-bit `bound_constraint_with_offset` asymmetry that record flagged **is** now a code-level candidate (P3-1) |
| ACIR-gen | quadratic ACIR expression growth in loops (#4629; `as_witness` is the manual workaround) | #4629 / #6539 (dup) | High value (quadratic → linear on loop-heavy circuits); design-first, high risk — **DESIGN NOTE ALREADY DELIVERED (`sq-felqr`, program §10.6); not re-derived by P3** |
| ACVM | `acvm-repo/.../optimizers/general.rs:16` (on-the-fly term merge TODO), `optimizers/common_subexpression/` (CSE) | issue #10109 | Backend-level term merging + CSE run after ACIR-gen; unexamined at code level — **REFUTED (P3): the #10109 on-the-fly merge is largely already done in `Expression::add_mul` and is a compile-time concern with no circuit-output change; the one genuinely open gap (#10192, cross-opcode subset sharing) is *gates-risk-inverted*.** This layer is also where P3 found the discount that voids a whole family of SSA-level candidates |

The survey (`research/noir-optimization-program.md` §7) already **ranked** several
of these as future opportunities (rows 5, 6, 9, 10) and beaded them under
`sq-uuvac`. The paper's contribution is not to re-rank them but to **carry a
measured candidate through each un-examined stage while writing** — that is
exactly the "surface further optimisations in the course of developing it"
mandate, operationalised in bead P3 (§5).

## 3. Paper outline (venue-agnostic)

Modelled on the estate's existing measured-and-honest paper structure
(`site/papers/sparql-logic-bugs.typ`: technique re-derived → instrument →
evaluation-with-evidence-that-exists-today → threats → honest-status).

1. **Introduction / motivation.** Proving cost = backend gate count; a
   *compiler-level* optimisation multiplies across every circuit compiled, unlike
   a hand-optimised circuit. sparq's real ZK circuits (compose/filter/scan
   kernels, IEEE-754 double arithmetic, XPath string/numeric kernels) are the
   natural, non-toy subjects.
2. **Background.** The Noir → ACIR → UltraHonk lowering; what a gate is; the
   ACIR-opcode-vs-backend-gate divergence (why opcodes alone mislead); the SSA
   pass pipeline; the euclidean-division primitive that range checks, truncation,
   comparison, and signed math all lower to (why range/bit-width work has
   outsized leverage).
3. **Case studies (empirical core).** One per fresh PR (#13263-#13266), each as
   gap-on-HEAD (cited `file:line`) → rewrite → **soundness argument** (sound
   over-approximation; no global use of branch-local/predicated facts;
   `range_check`-elision restricted to `range_check`-derived facts; signed
   non-firing) → **measured before/after** (bb gates + ACIR) → corpus regression.
   jeswr's #12780/#12781/#12927 appear as the originating prior work the program
   re-derives (#13263) and complements.
4. **Survey methodology (methodological contribution).** The reproducible
   whole-compiler survey protocol: metric authority order, the pinned-toolchain
   corpus, the fork-CI arbiter caveat, and the AI-assisted upstream-contribution
   etiquette that the noir community's `CONTRIBUTING.md` requires (draft +
   disclosure + author-review-first). This is a transferable process, not a
   one-off.
5. **New opportunities surfaced while writing.** The §2.2 gap analysis carried to
   measured candidates: for each un-examined stage, the hypothesis, the code
   evidence, and — where a `bb gates` win reproduces — a submitted draft PR. The
   honest negative results (candidates that did not reproduce a gate win, e.g. the
   #10159-shaped comparison-lowering trap) belong here too.
6. **Evaluation.** Aggregate measured results across the corpus; the
   canonical-vs-indicative split; the external upstream `noir-gates-diff` arbiter;
   **threats to validity** (work-box measurement, focused fixtures, fork-CI
   coverage, floor effects on small circuits).
7. **Related work.** Compiler optimisation for arithmetic/ZK circuits — Circom,
   the CirC / zkLLVM line, Leo/Aleo, Noir/Aztec itself — positioning this as
   *measured, upstreamed, single-compiler pass-level* work rather than a new
   whole-circuit-compiler design.
8. **Limitations & honest status.** The drafts are unmerged and awaiting @jeswr +
   upstream review; measurements are work-box (deterministic counts reproducible,
   wall-clock indicative); these are size opts and make no ZK-soundness/privacy
   claim; some wins are floor-limited on small circuits.
9. **Conclusion.**

## 4. Measurement plan

### 4.1 Metric authority (deterministic-first)

1. **Backend gate count** — `bb gates -s ultra_honk -b target/<pkg>.json`,
   `circuit_size`. Ground truth for proving cost. **Deterministic under a pinned
   `nargo`+`bb`** ⇒ **canonical** (reproducible, environment-independent), even
   though it was captured on the work box.
2. **ACIR opcode count** — `nargo info --json`, per entry point. Deterministic ⇒
   canonical, but **never a standalone win claim** (opcodes ≠ gates).
3. **Compile time / peak memory** — wall-clock `nargo compile`. **Non-canonical
   work-box** ⇒ **indicative only**; used solely to show a size win did not blow
   the compile-time budget (the #12927 O(n²) lesson).
4. **Upstream `noir-gates-diff` CI** — the 0.9-quantile gates-diff on pinned real
   Aztec circuits is the **external arbiter**. Fork PRs get **no** sticky comment
   until @jeswr flips a draft to ready, so this evidence is *pending* for the four
   drafts — a first-class limitation, not a gap to paper over.

### 4.2 Corpus (subjects)

noir `test_programs/benchmarks` (`sha512_100_bytes`, `semaphore_depth_10`,
`bench_eddsa_poseidon`, `bench_poseidon2_hash_100`) + real sparq ZK circuits: the
`zk/compose` scan/filter bins and IEEE-754/XPath kernels. Gotcha to encode in the
harness: `nargo info` returns `{"programs":[]}` for `type = "lib"` packages, so
library kernels (ieee754, xpath) must be measured via **bin probe wrappers** (as
the existing `~/noir-optim-workspace/probes/` do), never on the libs directly.

### 4.3 Determinism + evidence discipline

- Record the exact `nargo`+`bb` versions with every table (the current numbers are
  `bb 5.0.0-nightly.20260522`); a gate/opcode count is only comparable within one
  pinned toolchain.
- Route **every** figure the paper prints through the paper-factory evidence-JSON
  (`site/src/data/paper-evidence.json`) with `environment = canonical` for
  deterministic gate/opcode counts and `environment = indicative` for work-box
  compile-time, so the build-time honesty gate (`build-papers.mjs`) binds the
  claim. No raw literal in `.typ` prose.
- **Regression protocol (already the program's rule):** recompile the full corpus
  per candidate and diff ACIR opcodes; any unexplained increase is a stop signal.

### 4.4 The reproducibility gap this closes

The current measurement scripts (`measure_rrc_final.sh`, `compare_rrc.py`, the
probe bins) live only in `~/noir-optim-workspace`, **outside the repo**, and are
ephemeral. A paper whose empirical section is reproducible needs a committed
harness that regenerates the tables from a pinned toolchain — bead P1 (§5).

## 5. Phased plan (future beads, children of `sq-1j5ow`)

Created under `sq-1j5ow` via `bd` (parent-child), all P3 to respect the
maintainer's explicit "LOW PRIORITY, defer until little else to do" on the parent.

1. **`sq-i50o4` — P1: commit a reproducible measurement harness.** Port the
   ephemeral workspace scripts into a committed, pinned-toolchain harness that
   regenerates the ACIR + `bb gates` table for the four fresh-PR fixtures + the
   sparq corpus and emits the papers evidence-JSON (canonical for deterministic
   counts, indicative for compile-time). Prerequisite for a reproducible empirical
   section.
2. **`sq-mc7ft` — P2: draft the case-studies section.** One case study per fresh
   PR (#13263-#13266) with the measured table, soundness argument, and corpus
   regression, sourced through the P1 harness; fold in jeswr's #12780/#12781/#12927
   as originating prior work. Depends on P1.
3. **`sq-mtolx` — P3: gap-analysis / new-opportunities pass.** Examine the §2.2
   un-examined stages at code level; for each candidate that reproduces a `bb
   gates` win via the P1 harness, spawn a measured-PR child bead **under
   `sq-uuvac`** (not from a paper bead). Produces the paper's "new opportunities"
   section — the "surface further optimisations while writing" mandate.
   **CODE-LEVEL HALF DELIVERED (2026-07-29):
   `research/noir-optimization-new-opportunities.md` — all ten §2.2 rows examined
   against pin `8f33502e`, five refuted, one corrected, five candidate specs
   written, plus a draft of the paper's §5 prose. The MEASURED half is NOT done
   and **zero beads were spawned**: the spawn gate requires a reproduced `bb`-gates
   win, and P1 has not landed while `bb` is unavailable in the measurement
   environment (as it was in program §10.8/§10.9). The bead stays **open**,
   blocked on `sq-i50o4`.**
4. **`sq-ome8p` — P4: survey-methodology + related-work sections.** The
   reproducible survey protocol + upstream etiquette as the methodological
   contribution; a lit survey of compiler optimisation for ZK/arithmetic circuits
   for related work.
5. **`sq-99zhs` — P5: instantiate the Typst paper + evidence wiring + venue.**
   Create `site/papers/noir-compiler-optimization.typ` + `.refs.yml` +
   evidence-JSON wiring + `papers.ts` row, passing the paper-factory honesty
   gates; route every figure through a `headline()`/`ev()` accessor; select a
   venue. Depends on P1-P4.

## 6. Open questions for the maintainer

1. **Venue framing.** This is single-compiler, pass-level, *measured + upstreamed*
   work — closer to a tools/experience or artifact-track paper than a
   novel-algorithm ZK-systems paper. Which venue class, and how much to foreground
   the AI-assisted upstream *methodology* itself as a contribution (it is a genuine
   contribution, but also a sensitivity given noir `CONTRIBUTING.md`'s stance on
   AI-generated PRs)?
2. **Publish-before-merge risk.** The strongest evidence (upstream
   `noir-gates-diff` on real Aztec circuits) only exists once you flip the drafts
   to ready. Should the paper wait for those canonical external numbers, or ship
   on work-box-canonical + fork-CI numbers with the arbiter caveat? Presenting an
   unmerged, possibly-rejectable optimisation as a result is the #10159-shaped
   risk to avoid.
3. **Scope of the "new opportunities".** Include a §5 candidate only once it has a
   reproduced `bb gates` win (and ideally an accepted PR), or present the full
   set — including honest negative results — as "surfaced, measured, submitted"?
4. **Authorship / attribution.** How to attribute across your three originating
   PRs, the program's four fresh drafts, and the agent methodology?

## 7. Status & honesty

- **DOC-ONLY design-for-review.** No paper `.typ` source is written here; no
  upstream PR is opened from this bead. The only artifacts produced are this
  record and the five `sq-1j5ow` child beads.
- **None of the seven PRs is merged.** The four fresh drafts (#13263-#13266) are
  **open, awaiting @jeswr's author review**; the paper must never present any as
  landed or maintainer-accepted.
- **Measurements are real but work-box.** The bb-gates and ACIR counts are genuine
  runs (`bb 5.0.0-nightly.20260522`); deterministic counts are reproducible and
  treated as canonical, wall-clock/memory as indicative. The upstream CI gates-diff
  remains the external arbiter and is pending for the drafts.
- **No ZK claim.** These are circuit-*size* (proving-cost) optimisations; they make
  and require no zero-knowledge privacy or soundness property.
