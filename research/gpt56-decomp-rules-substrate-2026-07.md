# GPT-5.6 decomposition: shared rules kernel (Datalog / N3 / RIF) — 2026-07

> 🤖 **SPARQ agent** (Claude Fable 5, architect stage). Design record for the
> de-risked decomposition of the sq-6tykl.3 rules program under the maintainer
> direction in issue #1993. Companion record:
> `gpt56-decomp-workbench-parity-2026-07.md`. Beads carry the per-child specs;
> this record holds the decisions.

## 1. Maintainer direction (issue #1993, 2026-07-11)

1. N3 and RIF must **share the same stratification checker and evaluator** as
   the native Datalog dialect (PR #1992, `crates/sparq-reason/src/datalog/`).
2. N3 must be **expressive enough to be a superset** of the native dialect and
   RIF; any construct N3 cannot express → a clear, concise issue on the
   w3c/N3 CG repo stating what is missing and that it was discovered
   implementing RIF/RDFox rules support.

## 2. Current estate (ground facts, origin/main 517f8e54a)

Three rule paths exist in `sparq-reason`, already sharing the substrate join
kernels (`sparq_substrate::join::{build_table, hash_probe_serial}`, `Row`/`NO_ID`)
via the same thin layout-adapter pattern — but NOT a checker or evaluator:

| path | IR | checker | evaluator | extras the others lack |
|---|---|---|---|---|
| `datalog/` (feature `datalog`) | `DTerm/Atom/AggAtom/Rule` | **checked** `stratify.rs` (Ullman fixpoint, `Key::{Pred,Class,TypeAny}` granularity, loud rejection) | naive rounds per stratum (`eval.rs`) | NAF (single atom), `AGGREGATE COUNT`, numeric `FILTER` (substrate `Dec`) |
| `n3/compiled.rs` (feature `compiled-rules`) | `CTerm/Step/CompiledRule` | **none** — `log:notIncludes` stratification is documented *caller discipline* (module doc lines 35–39) | **semi-naive** (`join_steps` delta positions) | builtins (`string:*`, `log:uri`, id-compare), variable predicates, multi-pattern NAF |
| `rif.rs` + `rif_xml.rs` (features `rif-core`/`rif-xml`) | `rif::{Document,Rule,Atom,Term}` | not needed (monotone Horn) | lowers to the N3 model, runs `reason_n3` | frame/member/subclass atoms, 16 builtins (lowered to N3 `math:`/`string:`/`list:`) |

Numeric seam: datalog FILTER already uses the substrate `Dec`; the N3 chainer
(and therefore RIF builtins) still use the private `NumVal` tower — adoption is
the already-open bead **sq-pbz04.5.1** (not re-cut here).

## 3. Decision: converge on the datalog module as the *rules kernel*

The datalog module's IR + checker + evaluator becomes the dialect-neutral
**rules kernel**; Datalog surface syntax, N3, and RIF become thin front-ends
that lower onto it. Rationale:

- The checker only exists there, and #1993 makes the *checked* posture the
  target for all dialects (N3's caller discipline is exactly what we're
  retiring).
- Its IR is the strict-semantics core (NAF + aggregates + filters); the
  compiled-N3 extras (builtins, variable predicates, multi-pattern NAF) are
  additive slots, whereas retrofitting aggregates/checking onto compiled-N3
  would rebuild the kernel inside a dialect.
- RIF already lowers onto another model today (the N3 chainer), so re-pointing
  its lowering is a bounded, conformance-ratcheted change (W3C RIF WG floor).

Convergence is staged so every GPT-5.6-implementable step is single-crate,
non-soundness-sensitive, and differentially tested against the existing naive
oracle (`datalog/oracle.rs`) — the soundness-sensitive lowerings stay
architect/Fable-tier:

1. **Checker-core extraction** (new bead, GPT-5.6): factor the Ullman fixpoint
   out of `datalog::stratify` into a dialect-neutral core
   (`stratify_edges(n_nodes, pos_edges, neg_edges, name_of) -> Result<Vec<usize>, String>`);
   `datalog::stratify` becomes a thin adapter. Pure refactor: every existing
   checker test passes verbatim.
2. **Semi-naive datalog eval** (sq-8sve7, GPT-5.6): adopt the `join_steps`
   delta discipline compiled-N3 already uses, inside `datalog/eval.rs`.
   Correctness gate = the existing differential suite; efficiency gate = a
   **deterministic** tuples-considered counter (work-box wall-clock is
   non-canonical), asserted strictly smaller than naive on a chain fixture.
3. **SUM/MIN/MAX/AVG** (sq-citho, GPT-5.6, after 2 — same-file sequencing):
   value slot on `AggAtom`, inputs via the substrate `Num` tower.
   **Overflow semantics decided here**: follow `Num::binop` — exact i64/`Dec`
   paths, fall back to `xsd:double` on exact-arithmetic overflow; `AVG` of
   integers yields `xsd:decimal` (SPARQL-consistent); non-numeric input value
   fails the row (fail-closed, same posture as FILTER). Oracle extended in
   i128 so the differential stays independent.
4. **Fragment extensions** (sq-a7bmo, GPT-5.6, after 3): `NOT` over
   conjunctions (parity with compiled-N3 multi-pattern `log:notIncludes`),
   `COUNT(DISTINCT ?v)`, float-aware FILTER via `Num::cmp_relational`,
   variable predicates via a conservative ⊤ dependency node.
5. **N3 adopts the checker** (sq-pi2k0, GPT-5.6, after 1): `n3::compiled`
   builds its dependency graph (patterns → positive edges, `NotIncludes` →
   negative edges, variable predicate → ⊤ node) and rejects unstratified rule
   sets loudly. Fail-closed-only behaviour change: programs whose semantics
   were previously undefined-divergent now error; stratified sets unchanged.
6. **N3 lowering onto the kernel** (new bead, **architect/Fable only**):
   builtin steps + variable predicates as kernel slots, `n3::compiled` eval
   re-pointed; requires a semantics-equivalence argument vs `reason_n3`
   (soundness-sensitive).
7. **RIF lowering onto the kernel** (new bead, **architect/Fable only**,
   after 6): `rif::closure` targets the kernel instead of the N3 chainer; the
   W3C RIF WG Core floor (`rif_wg_core_suite.rs`) is the gate. Composes with
   sq-pbz04.5.1 (substrate numeric adoption) — sequence, do not parallelize.

Steps 1–5 are all `sparq-reason`-only, behind existing opt-in features, and
leave every default-feature surface byte-identical.

## 4. N3-superset gap analysis (the #1993 ask)

Checked against the compiled subset's exclusion list and the RIF builtin
lowering table:

- **RIF-Core**: no gap found. Frames/membership/subclass lower to triples;
  all 16 RIF builtins already lower to N3 `math:`/`string:`/`list:` builtins;
  RIF is monotone so needs neither NAF nor aggregates.
- **RDFox Datalog `NOT`**: expressible — `log:notIncludes` over a formula
  covers single- and multi-pattern NAF (store-scoped reading is a documented
  sparq interpretation; not a missing *construct*).
- **RDFox `AGGREGATE … ON … BIND f(?v) AS ?x`**: **no dedicated primitive**, but
  **NOT an N3 expressivity gap** (sq-6tykl.3.4, 2026-07-27) — verified against the
  live Notation3 Language spec and the Notation3 Builtin Functions **Final CG
  Report (3 June 2026)**: neither contains the word "aggregate", and there is no
  aggregate builtin family. Aggregation is nonetheless *expressible*, by
  composition through the formula-/list-valued `log:collectAllIn` + list
  builtins — which is exactly what **our** compiled/stratified subset excludes,
  so the blocker is a deliberate profile restriction on the sparq side, not a
  limit of N3. Refinements to the original reading, all recorded in
  `n3-aggregate-gap-upstream.md`: **grouping is NOT missing** (enclosing-body
  variables are the `ON` key); SUM (`math:sum`) and COUNT (`list:length`) have
  builtins, AVG composes in three steps, and **MIN/MAX have no builtin** but are
  still encodable via recursion or `list:member` + negation-as-failure. Venue
  correction: the CG repo is now **`w3c-cg/N3`**, and upstream
  **PR #119** already proposed `list:min/max/sum/avg/…` in 2023 and was closed
  unmerged with zero comments. Issue drafted, **not filed** — @jeswr review
  gate per `AGENTS.md` § *Upstream contributions*.

## 5. Reversal cost

The kernel stays `pub(crate)`; the only public surfaces remain the existing
dialect entry points. If the maintainer prefers compiled-N3 as the kernel
instead, steps 1–5 still stand (checker core, semi-naive, aggregates, fragment
parity are prerequisites under either direction); only steps 6–7 re-plan.
