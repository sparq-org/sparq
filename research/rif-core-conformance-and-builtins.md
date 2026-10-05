# RIF-Core: conformance arm + builtins over the shared numeric tower (sq-pbz04.5) [FABLE-5]

> **Status: DESIGN / DECOMPOSITION RECORD — no implementation in this PR.** Authored by
> Claude Fable 5 as the architect pass for epic **sq-pbz04.5** (parent sq-pbz04, program
> sq-6tykl, `research/reasoner-federation-program.md`). This record corrects the epic's
> premise where it is stale, pins the per-builtin soundness arguments, and decomposes the
> workstream into six disjoint child beads the fleet implements. Everything below is
> **designed-only** unless explicitly marked BUILT.

## 1. Ground truth — what is actually BUILT (verified against `origin/main`)

The epic said "there is no RIF conformance-suite arm today". That is **half right**:

| Surface | State | Where |
|---|---|---|
| RIF-Core in-engine model + Datalog-safety validation + N3 lowering + closure | **BUILT** | `crates/sparq-reason/src/rif.rs` (`rif-core` feature; 16 builtins; `Document::validate` enforces range-restriction left-to-right, arity, builtin-in-head rejection) |
| RIF **expressivity** ratchet (self-asserting, sparq-extension-labelled) | **BUILT** | `crates/sparq-conformance/tests/rif_core_suite.rs` (`RIF_CORE_FLOOR`, mirrored textually by `crates/sparq-conformance/tests/scoreboard_floors.rs`; honestly framed as NOT a W3C-suite or SPARQL-RIF-entailment claim) |
| RIF/XML presentation-syntax importer | **ABSENT** | listed in `rif::UNIMPLEMENTED` — the front-end takes the in-engine model only |
| W3C **RIF WG test-suite** arm (the actual Core test cases) | **ABSENT** | nothing consumes the RIF WG test repository |
| Builtins over the **shared** `sparq_substrate::numeric` tower | **ABSENT** | RIF builtins lower to N3 `math:`/`string:`/`list:` builtins, which the chainer evaluates with its own **private** `NumVal` tower (`n3/mod.rs`, `enum NumVal { Int(i128), Dec(i128,u32), F64 }`, EYE-parity semantics) — this private tower is the real seam-2 gap |
| Value-space comparator (seam-2 remainder) | **NUMERIC half LANDED** (sq-v5evr `Num::cmp_relational`, #1646), adopted by the RIF Equal path in sq-anyad; the **non-numeric** half is still OPEN | `sparq-substrate::numeric` — distinct ground *numeric* constants are now decided (value-equal → eliminated, value-unequal → vacuous rule); boolean/string/temporal literal equality stays deferred behind a non-numeric value-space comparator |

So the two genuinely greenfield items are (i) the RIF/XML importer + the W3C RIF WG
suite arm on top of it, and (ii) the seam-2 numeric-tower adoption *inside the N3
chainer* (which serves RIF transitively, because RIF builtins lower to N3 builtins).

## 2. Seam-2 decision — where "builtins over the shared tower" actually lands

The epic directs: RIF builtins implemented over the shared `sparq-substrate` numeric
tower, "NOT a private builtin evaluator". Three options were weighed:

- **A (CHOSEN): adopt `sparq_substrate::numeric::{Num, Dec}` as the arithmetic core
  inside the N3 chainer**, behind a thin EYE-compat adapter. RIF builtins lower to N3
  builtins, so one adoption serves *both* dialects with a single evaluation path. The
  EYE-specific behaviours stay at the adapter edges and are preserved exactly:
  lexical-shape coercion of plain strings, EYE result rendering (`numval_term`,
  including whole-`f64` → `xsd:integer`), `math:remainder`'s divisor-sign integer
  semantics. One representation wrinkle is load-bearing: the chainer's `Int` is
  **i128** while substrate `Num::Int` is **i64** — integers outside i64 range must map
  to substrate `Dec { mant: i128, scale: 0 }` (exact, overflow-checked with f64
  fallback), and the differential test must cover `> i64::MAX` arithmetic.
- **B (REJECTED): a RIF-private evaluator** over the substrate. Violates the epic's
  explicit seam-2 discipline and creates a *third* arithmetic path (engine, N3, RIF).
- **C (the documented escape hatch): reasoned non-adoption**, mirroring EL's
  saturation-join precedent. If the behaviour-neutrality differential (old `NumVal`
  vs substrate-backed) reveals an irreconcilable semantic mismatch on the pinned
  floors, the honest outcome is a written non-adoption rationale in `n3/mod.rs`, not a
  behaviour change smuggled through a refactor.

**Invariant for A:** byte-identical N3 closures — the N3/EYE differential floors, the
N3 expressivity floors, and `RIF_CORE_FLOOR` are all unchanged. This is a pure
refactor under the program's behaviour-neutral rule; any floor movement fails it.

**Dependency honesty:** this makes `sparq-substrate` (with only the `numeric` feature)
a *non-optional* dependency of `sparq-reason`. Deviation from "opt-in feature
throughout" is deliberate and argued: `sparq-reason` is itself the opt-in surface and
is never in the wasm dependency graph; `substrate::numeric` is a leaf module already
compiled in every workspace build (the engine consumes it); and a feature-gated dual
evaluator (NumVal fallback + substrate path) would be *worse* — two arithmetic cores
is exactly the divergence risk seam 2 exists to kill. The lean-core invariant
(`sparq-core`/`sparq-engine` defaults, wasm byte floor) is untouched.

## 3. Builtin surface — sound-to-map now vs honestly deferred

RIF-Core imports its builtins from XPath/XQuery 1.0 F&O via RIF-DTB. The existing 16
builtins already map (numeric add/sub/mul/div + the five comparisons, string
contains/starts-with/ends-with/concat/length, list contains/length). The expansion
below maps **only where the DTB (F&O) semantics and the N3 target's (EYE-parity)
semantics coincide on the RIF-Core value space** — every non-coincidence is a
documented deferral, never a close-enough mapping.

### 3.1 Map now (each with its equivalence argument)

| RIF builtin | N3 target | Soundness argument |
|---|---|---|
| `pred:numeric-not-equal` | `math:notEqualTo` | both are numeric value-space inequality over the same promotion tower; the chainer's `MathNe` exists and is exercised by the N3 suites |
| `func:upper-case` | `string:upperCase` | XPath `fn:upper-case` default Unicode case mapping ≙ Rust `str::to_uppercase` (same default, no locale/tailored mappings on either side); caveat recorded that neither side implements locale-tailored casing |
| `func:lower-case` | `string:lowerCase` | symmetric to upper-case |
| `func:encode-for-uri` | `string:encodeForUri` | the chainer's builtin is *documented as exactly* XPath `fn:encode-for-uri` (RFC 3986 unreserved-set, uppercase hex) — definitionally the same function |
| `func:concatenate` (lists) | `list:append` | both concatenate a sequence of lists into one list, order-preserving; the chainer's `Append` takes `( list… )` which matches DTB's variadic signature |

### 3.2 Deferred — with the precise reason (honest incompleteness, not gaps to paper over)

| RIF builtin | Why NOT mapped (the unsound-if-mapped argument) |
|---|---|
| `func:numeric-integer-divide` | F&O `op:numeric-integer-divide` **truncates toward zero**; EYE/N3 `math:integerQuotient` is **floor** division. They differ on any negative operand (`-7 idiv 2` = `-3` vs `floor(-3.5)` = `-4`). Mapping would derive wrong values silently. Needs either a truncating N3-side builtin or RIF-side compilation — deferred, tracked in `UNIMPLEMENTED`. |
| `func:numeric-mod` | F&O `op:numeric-mod` result follows the **dividend** sign; `math:remainder` is integer-only with **divisor**-sign semantics. Mixed-sign operands diverge. Same deferral rationale. |
| `pred:matches` | XPath/XSD regex dialect ≠ Rust `regex` dialect (e.g. XSD character-class subtraction `[a-z-[aeiou]]`). A mapping cannot fail closed on dialect-divergent patterns without a real XSD-regex front-end. Deferred; a future opus bead may add a translated-subset with fail-closed detection. |
| `pred:boolean-equal`, `pred:literal-not-identical`, equality of *distinct* value-equal constants | **PARTLY RESOLVED (sq-anyad).** The NUMERIC half is implemented: distinct ground numeric literals are decided by the shared `Num::cmp_relational` (sq-v5evr, #1646). The NON-numeric half (booleans, strings, temporal literals) needs value-space equality beyond the numeric tower and stays deferred, fail-closed with `DistinctGroundEqual`. |
| guard predicates (`pred:is-literal-integer`, `pred:is-literal-string`, …) | would require *inventing* non-EYE N3 builtins, polluting the chainer's EYE-differential story. Deferred; the conformance arm skips guard-using cases under a named category. |
| `func:substring`, `func:substring-before/-after`, `func:string-join`, `func:compare` | no semantically-matching N3 target exists today (`string:scrape` is regex capture, not substring). Deferred rather than approximated. |
| `func:get`, `func:sublist`, `func:reverse`, `func:index-of`, `func:insert-before`, `func:remove`, `func:union`, `func:distinct-values`, `func:except`, `pred:is-list` | no 1:1 N3 target; multi-triple lowerings (e.g. `list:iterate` + filter) change the producer/consumer shape and its safety analysis — not worth the risk for the test coverage they buy. Deferred. |
| date/time/duration builtins | no shared temporal tower exists (the chainer's `time:` support is partial and EYE-shaped; the substrate has no temporal module). Deferred wholesale — a temporal seam is its own future design record. |

`func:count` is already covered (`Builtin::ListLength` lowers to `list:length`).

### 3.3 Equality-atom audit (a Core-fidelity bug found during grounding)

Two defects in the current `Equal` handling, both confined to `rif.rs`:

1. **RIF-Core forbids equality in rule conclusions** (it is one of Core's syntactic
   restrictions relative to BLD). The current model *accepts* `Equal` in a head and
   lowers it to an `owl:sameAs` triple — which the chainer then treats as an inert
   triple (no substitution semantics), so the front-end both over-accepts non-Core
   input and under-delivers BLD semantics. Fix: `validate()` rejects equality in
   conclusions with a new `RifError` variant (fail-closed, Core-faithful).
2. **Body `Equal` conflates vocabulary with built-in equality.** Lowering `a = b` to
   an `owl:sameAs` triple *pattern* means (i) ground `t = t` is not derivable unless
   someone asserted a sameAs triple (under-derivation vs RIF semantics, where `t = t`
   is always true), and (ii) any asserted `owl:sameAs` vocabulary triple satisfies a
   RIF equality atom (an unlicensed conflation). Fix directions, in order: ground
   syntactically-identical terms → trivially true (eliminated at lowering); variable
   equality (`?x = t`) → compile-time substitution (standard unification, sound);
   ground *distinct* constants → **reject fail-closed pending sq-v5evr** (value-space
   equality), with the rejection reason naming the deferral. This changes closure
   behaviour, so the expressivity suite's equality axis moves **honestly** (floor
   updated with the change, in the same PR).

## 4. The conformance arm — W3C RIF WG test cases, honestly scoped

**What we claim:** an arm over the **W3C RIF Working Group test cases, Core subset**,
driven end-to-end through the real path (RIF/XML import → `validate` → `closure` →
conclusion check). **What we still do not claim:** the SPARQL **RIF entailment
regime** (`sparql11/entailment` `rif01..rif06` — a strictly larger integration through
the SPARQL protocol with externally-referenced rule documents); it remains a tracked,
named out-of-scope item exactly as `rif_core_suite.rs` frames it today.

Design points, each load-bearing for oracle soundness:

- **Categories:** `PositiveEntailmentTest` (premise ⊨ conclusion),
  `NegativeEntailmentTest` (premise ⊭ conclusion), `PositiveSyntaxTest` /
  `NegativeSyntaxTest` (importer accept/reject).
- **The NET vacuity rule (the one subtle oracle-correctness point):** a
  NegativeEntailmentTest passes only when the premise **imported, validated, and
  closed successfully** AND the conclusion is not satisfied. A premise that failed to
  import (unsupported construct) is a **SKIP**, never a pass — otherwise every
  unsupported feature would launder into a vacuous "not entailed". Fail-closed
  everywhere: unknown XML elements, `Import` directives, non-Core dialect features,
  and unknown `External` IRIs each map to a *named* skip category
  (`skip:non-core`, `skip:imports`, `skip:unsupported-builtin`,
  `skip:condition-shape`), printed per-run so incompleteness is legible.
- **Conclusion matching:** ground conclusions and existential-free conjunctions first;
  numeric literals compare **value-aware** via `sparq_substrate::numeric::Num` (so an
  entailed `"2.0"^^xsd:decimal` is not a false negative against a derived
  `"2"^^xsd:decimal`); other typed literals compare lexically+datatype. Conclusions
  outside this shape → `skip:condition-shape`.
- **Or/Exists in conditions are IN Core** and are handled at *import* time by sound,
  standard transformations: body `Or` splits the rule (one rule per disjunct — the
  classic Lloyd–Topor step, monotone-Horn-preserving); body `Exists` variables become
  ordinary body variables under the existing range-restriction validation. The
  in-engine `rif.rs` model stays untouched — the desugaring is importer-owned. (The
  current `UNIMPLEMENTED` note listing `Or` as "larger-dialect" is *stale* — Core's
  condition language includes disjunction; the importer bead corrects the note.)
- **Fixtures + licensing (two-path, license-gated):** the RIF WG test cases are not in
  `w3c/rdf-tests`. The bead must **verify the license first**: if W3C's terms permit
  redistribution (Test Suite / Software-and-Document licence family), vendor the Core
  subset under `tests/w3c/rif/` with a provenance note, mirroring the vendored-W3C
  precedent; otherwise use the established external-checkout + self-SKIP pattern the
  D-entailment lane already uses (`tests/w3c/rdf-tests` fetched by script, lane skips
  offline). No fabricated fixtures in either path.
- **Floor mechanics:** a pinned calibrated floor const in the new lane, textually
  mirrored in `crates/sparq-conformance/tests/scoreboard_floors.rs` (the established guard), tallied as a
  **standards-suite lane with an honest denominator** (W3C-authored cases; the skip
  taxonomy is the denominator's honesty), distinct from the self-asserting
  expressivity ratchet, which stays and keeps its sparq-extension label.

### 4.1 Capture-avoidance in the Exists-flatten desugaring (sq-pbz04.5.3 post-verify)

An Opus adversarial-verify on sq-pbz04.5.3 confirmed that the original Exists-flatten
implementation suffered **variable capture** in two distinct patterns:

1. **Quantifier shadow** — `Forall ?x: h(?x) :- And(p(?x), Exists ?x(q(?x)))`. The
   outer `Exists ?x` re-declares a name already in scope as a universally-quantified
   variable. Without renaming, the body after flattening contains two atoms both
   referencing `Var("x")` — no way to tell universals from existentials apart, and the
   existential's binding scope collapses into the universal's scope.

2. **Sibling reuse** — `Forall ?x: h :- And(Exists ?y(p(?x,?y)), Exists ?y(q(?x,?y)))`.
   Two sibling `Exists` nodes each declare `?y`. Without renaming, both atoms use
   `Var("y")` after flattening — a single binding of `y` during forward chaining
   satisfies both atoms simultaneously, which is semantically incorrect (they must use
   DIFFERENT witnesses).

The **fix** (sq-pbz04.5.3, applied and shipped) is **unconditional alpha-renaming**:
every Exists-declared variable is renamed to a fresh name in the `__ex{N}` reserved
namespace before the Exists wrapper is dropped. "Unconditional" means even non-colliding
variables are renamed — this uniformly handles both patterns above without any
case-analysis on whether a collision exists. Freshness is verified against the complete
variable universe (universally-declared vars + all Exists-declared vars + previously
generated fresh names) so no `__ex{N}` name can collide with anything in scope.

Renaming is innermost-first (via DFS pre-order on sub-conditions before the enclosing
Exists), so the **innermost binder wins** when the same name is redeclared in nested
Exists nodes — consistent with standard lexical-scope semantics.

The implementation carries a **mutation-check proof** (`test_capture_mutation_check`):
calling `expand_body` without prior `alpha_rename_cond` produces provably duplicate body
vars for both capture patterns, and the test asserts that. Disabling
`alpha_rename_cond` in `parse_implies` makes `test_alpha_rename_shadow` and
`test_alpha_rename_sibling` RED — the alpha-renaming step is the load-bearing mechanism.

## 5. Decomposition — six disjoint child beads

Wave order expresses real dependencies (shared files are serialized by `bd dep`; no
two *parallel* beads touch any common file).

| Bead | Wave | Crate | Tier | Exclusive files | One-line scope |
|---|---|---|---|---|---|
| **sq-pbz04.5.1** (B1 tower adoption) | 1 | sparq-reason | opus | `crates/sparq-reason/src/n3/mod.rs`, `crates/sparq-reason/Cargo.toml` | replace the private `NumVal` arithmetic core with `sparq_substrate::numeric` behind an EYE-compat adapter; byte-identical floors; escape hatch = documented non-adoption |
| **sq-pbz04.5.2** (B2 builtin mapping) | 1 | sparq-reason | sonnet | `crates/sparq-reason/src/rif.rs`, `crates/sparq-conformance/tests/rif_core_suite.rs` | add exactly the §3.1 table (5 builtins) + record the §3.2 deferrals; expressivity floor rises |
| **sq-pbz04.5.3** (B3 RIF/XML importer) | 2 | sparq-reason | opus | `crates/sparq-reason/src/rif_xml.rs` (new), `crates/sparq-reason/src/lib.rs`, `crates/sparq-reason/Cargo.toml` | new `rif-xml` feature (+ workspace `quick-xml`, optional dep); Or-split + Exists-flatten desugaring; fail-closed taxonomy |
| **sq-pbz04.5.4** (B5 Equal-atom audit) | 2 | sparq-reason | opus | `crates/sparq-reason/src/rif.rs`, `crates/sparq-conformance/tests/rif_core_suite.rs` | the §3.3 fix: reject `=` in conclusions; ground-identity + variable-substitution body semantics; distinct-constant value equality fail-closed pending sq-v5evr |
| **sq-pbz04.5.5** (B4 WG suite arm) | 3 | sparq-conformance | opus | `crates/sparq-conformance/tests/rif_wg_core_suite.rs` (new), `crates/sparq-conformance/Cargo.toml`, `crates/sparq-conformance/tests/scoreboard_floors.rs`, fixtures/fetch script | the §4 arm: categories, NET vacuity rule, value-aware matching, license-gated fixtures, pinned floor |
| **sq-pbz04.5.6** (B6 lowering-boundary docs) | 3 | sparq-reason (docs) | haiku | `crates/sparq-reason/README.md`, `skills/inference/SKILL.md` | document the accepted safety class, the lowering boundary, and the deferral table (epic item c); readme-template gate respected |

Dependency edges (all real, no artificial serialization; wired via `bd dep`):

```text
.1 ──→ .3         (Cargo.toml is .1's in wave 1; .3 edits it after)
.2 ──→ .3         (importer maps External IRIs onto the post-.2 Builtin enum)
.2 ──→ .4         (same files: rif.rs + rif_core_suite.rs — NON-parallel by design)
.3, .4 ──→ .5     (the arm consumes the importer + the corrected Equal semantics)
.2, .3, .4 ──→ .6 (docs describe the final state)
```

Cross-epic shared-file cautions (outside this epic's control, flagged for the
scheduler): `crates/sparq-conformance/tests/scoreboard_floors.rs` is also a target of the EL arm bead
sq-pbz04.2.4; `crates/sparq-reason/src/lib.rs` may also gain a module line from sq-pbz04.1.2. Both are
tiny additive blocks — rebase-trivial, but the beads note them.

## 6. Non-goals and honesty ledger

- **No SPARQL-RIF entailment-regime claim** — tracked out-of-scope, unchanged.
- **No RIF-BLD/PRD claim** — the front-end stays Core (monotone Horn); NAF,
  production actions, function symbols, aggregation remain excluded by dialect.
- **No "complete RIF-DTB builtins" claim** — §3.2 is the deferral ledger; the
  conformance arm's skip taxonomy keeps the incompleteness measurable per run.
- **`integer-divide`/`mod`/`matches` are deferred _because mapping them would be
  unsound_** (sign-semantics and regex-dialect mismatches) — this record is the
  citable argument.
- **B1 is behaviour-neutral or it does not land** — floors byte-identical, with
  reasoned non-adoption as the honest fallback outcome.
- **sq-v5evr remains the gate** for value-space equality (boolean-equal,
  literal-not-identical, distinct-constant `=`); nothing here pre-claims it.
  *(Update, sq-anyad: sq-v5evr has since landed and the RIF Equal path adopted its
  `Num::cmp_relational` for the NUMERIC half only — distinct ground numeric constants
  are decided, everything non-numeric is still fail-closed. The remaining gate is a
  non-numeric value-space comparator.)*
- **This document contains no performance numbers**; quantitative floors live in the
  CI-checked ratchet files.
