# sparq-reason-ql

<p>
  <a href="https://crates.io/crates/sparq-reason-ql"><img src="https://img.shields.io/crates/v/sparq-reason-ql.svg" alt="crates.io"></a>
  <a href="https://docs.rs/sparq-reason-ql"><img src="https://docs.rs/sparq-reason-ql/badge.svg" alt="docs.rs"></a>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

**EXPERIMENTAL** opt-in **OWL 2 QL** (DL-Lite_R) query-rewriting reasoner for the
[sparq](../../README.md) RDF engine. It rewrites a conjunctive SPARQL query into a **union of
conjunctive queries** that, evaluated over the **unmodified data**, returns the **certain
answers** under the schema — no materialisation, no closure storage (Calvanese et al.,
*PerfectRef*, JAR 2007). It is a **query-rewriter**, not a materialiser: it reuses the engine's
query path (the spargebra rewrite seam), so the planner and executor are unaware of reasoning.

This is a **separate crate** and the rewriter is behind an **off-by-default `experimental`
feature** — the core engine and the wasm build carry zero QL code, deps, or cost by default.

> **Soundness boundary (read this).** PerfectRef is sound + complete only for **conjunctive
> queries** (and unions of them). Firing it on anything else silently mis-answers, so this crate
> is **FAIL-CLOSED**: `OPTIONAL` / `MINUS` / aggregation / a variable predicate / a **recursive,
> zero-length, or negated** property path (`+` `*` `?` `!p`) is **rejected as out-of-scope**,
> never rewritten. Non-recursive paths (`/` `^` `|`) ARE handled — desugared to a CQ/UCQ before
> rewriting (B5). The rewriter is validated against a **hand-checked DL-Lite_R oracle**: on the
> **formal DL-Lite_R suite** (`sq-g19x0`) it is **sound AND complete case by case** — now a
> **pinned floor** (`sparq-conformance`'s `ql_dllite_suite`, a *sparq-extension* ratchet, **not**
> a full-OWL-2-QL-conformance claim). The broader `pr:QL` `sparql11/entailment` set stays
> **EXPERIMENTAL / OutOfScope** (it mixes intensional cases outside sound rewriting).

## 🚀 Quickstart

```rust
# #[cfg(feature = "experimental")] {
use oxrdf::Triple;
use spargebra::SparqlParser;
use sparq_reason_ql::rewrite;
use std::str::FromStr;

// TBox: Manager rdfs:subClassOf Employee.
let tbox = vec![Triple::from_str(
  "<http://ex/Manager> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://ex/Employee> ."
).unwrap()];
let q = SparqlParser::new().parse_query(
  "SELECT ?x WHERE { ?x a <http://ex/Employee> }"
).unwrap();
let r = rewrite(&q, &tbox).unwrap();   // UCQ now also matches Managers (certainly Employees)
assert_eq!(r.report.disjuncts, 2);
# }
```

The CQ-shape gate, `as_conjunctive_query`, is **always present** (no feature needed): it
classifies any query as a CQ or `CqError::OutOfScope(reason)` and is the soundness keystone.

## ✨ Features

- **`rewrite` — PerfectRef baseline** *(opt-in `experimental`)* — rewrite + reduce saturation to a
  fixpoint over the positive DL-Lite_R inclusions: `rdfs:subClassOf` / `subPropertyOf`,
  `rdfs:domain` / `range` (`∃R ⊑ A`, `∃R⁻ ⊑ A`), `owl:inverseOf`, and unqualified `∃R`
  restrictions, with the existential **applicability condition** enforced explicitly.
- **`rewrite_production` — production path** *(opt-in `experimental`)* — PerfectRef **augmented**
  with bounded **tree-witness** folding (existential witnesses captured with no unbounded chase),
  then **UCQ-containment minimisation** (redundant disjuncts dropped by the homomorphism
  containment test). Returns the **same certain answers** as `rewrite` in a **smaller UCQ**.
- **Broadened sound fragment** — **(B1)** top-level UCQ; **(B2)** literal-object role atoms;
  **(B3)** `FILTER` / **(B4)** constant-only `VALUES` over distinguished-only vars (pass-through, applied **per-branch** in a multi-branch UCQ so a branch's filter never leaks — sq-sg542);
  **(B5)** non-recursive property paths (`/` `^` `|`) desugared to a CQ/UCQ before rewriting —
  fresh non-distinguished sequence intermediate (shared → a JOIN), inverse swap, alternation →
  UCQ branches [sq-pbz04.3.2]; **(Bbnode)** body blank nodes → fresh existential vars [sq-pbz04.3.6].
- **Intensional-atom guard (B6, always present)** — schema vocab predicates (`rdfs:subClassOf/
  subPropertyOf/domain/range`, all `owl:`) are **rejected**; annotation predicates admitted.
- **Fail-closed CQ-shape gate** *(always present)* — `OPTIONAL`/`MINUS`/paths/aggregation/
  variable-predicate queries **rejected** as `OutOfScope`, never mis-answered.
- **Query-rewriter seam** — emits a `Union`-folded UCQ as a `spargebra::Query`, run unchanged by
  the engine; no store or planner changes.
- **Honest fragment reporting** — TBox axioms outside DL-Lite_R are counted in
  `RewriteReport::skipped_axioms`, never silently applied.

> **Minimisation is fail-closed.** UCQ containment is **NP-complete**; the homomorphism search is
> **bounded**, and on an **undecided-within-budget** check the disjunct is **KEPT**, never dropped.
> Minimisation only ever removes a disjunct **proven contained** in a retained one — so it removes
> no answers. (Dropping a non-contained disjunct would be an unsoundness bug.)

**Scope (honest):** positive-inclusion rewriting + tree-witness folding + containment minimisation, plus opt-in **DL-Lite_R consistency checking** (`ql-consistency` feature, `check_consistency`, sq-p6yb7) — a violation query per captured negative inclusion (`owl:disjointWith` / `owl:propertyDisjointWith` / the subClassOf-complement `A rdfs:subClassOf [ owl:complementOf B ]`, which is exactly the DL-Lite_R negative inclusion `A ⊑ ¬B`), rewritten with the same PerfectRef machinery and evaluated over the data: **INCONSISTENT** iff some violation query matches (sound at any capture level); definitive **CONSISTENT** only for a fully-captured TBox with every negative axiom structurally captured; fail-closed **Unknown** otherwise (e.g. a NAMED-subject `A owl:complementOf B` — the equivalence `A ≡ ¬B`, stronger than a negative inclusion).
No qualified existentials; the **combined-approach / H-complete-ABox** lever was **evaluated + NOT adopted** (measured minimised UCQ ≤ 4 on the real corpus, no blow-up to amortise — `sq-pbz04.3.5`, profile in `examples/ql_rewrite_bench.rs`, rationale in `research/owl2-ql-cq-gate-broadening.md` §10).
The rewriter is oracle-tested two independent ways — syntactically against hand-derived UCQs (`tests/oracle.rs`) and, since sq-wxaas, **executably against an OWL 2 RL materialise-then-query baseline** (`tests/rl_baseline_differential.rs`): on RL ∩ QL fixtures the two strategies must return identical answers, and on an existential-generating axiom (no RL superclass form) QL must return a strict superset. So is the consistency check (`tests/consistency_oracle.rs`, hand-derived verdicts).

**Two conformance arms, two honesty stances (sq-qo1a9):**

- **GRADUATED — the formal DL-Lite_R certain-answer oracle.** On the hand-derived DL-Lite_R suite
  from `sq-g19x0` — every case a conjunctive query within sound rewriting — the rewrite is **sound
  AND complete case by case**: `rewrite_production`'s UCQ, evaluated over the **unmodified ABox**,
  returns **exactly** the hand-derived certain answers. This is pinned as a floor in
  `sparq-conformance`'s `ql_dllite_suite` (opt-in `ql-experimental`; `QL_DLLITE_FLOOR`). It is a
  **`sparq extension` row** in the central scoreboard, tallied **separately** and **never folded into
  the standards-conformance total** — there is no runnable normative W3C OWL 2 QL certain-answer
  suite, so this is honestly a faithful DL-Lite_R oracle floor, **not a full-OWL-2-QL-conformance
  claim**.
- **STILL EXPERIMENTAL — the broad `pr:QL` `sparql11/entailment` arm.** That set is **not** the
  formal DL-Lite_R suite: it mixes intensional / non-DL-Lite certain-answer cases the sound
  rewriting fragment cannot answer. So it stays experimental / OutOfScope (sq-kuvu3, opt-in
  `sparq-conformance/ql-experimental`, `tests/ql_experimental_arm.rs`): the harness reports —
  honestly — what it computes (fail-closed ABSTAIN / computed-equivalent evidence /
  computed-DIVERGENT gap), **never a graduated conformance pass**, and its rows are **never summed
  into any floor**.

Enable the crate with `sparq-reason-ql = { version = "0.1", features = ["experimental"] }`.

## 📚 Learn more

- **How-to** — [`skills/inference/SKILL.md`](../../skills/inference/SKILL.md) (QL section).
- **API reference** — [docs.rs/sparq-reason-ql](https://docs.rs/sparq-reason-ql).
- **Design** — [`research/owl2-el-ql-reasoning-spike.md`](../../research/owl2-el-ql-reasoning-spike.md)
  (the QL track) and [`research/reasoner-suite-on-substrate.md`](../../research/reasoner-suite-on-substrate.md)
  §2.5 (the PerfectRef trap + the phased plan; this crate implements phases Q1–Q3, and the formal
  DL-Lite_R certain-answer oracle has now graduated to a pinned `sparq extension` floor — sq-qo1a9).
- **Contribute** — [`AGENTS.md`](../../AGENTS.md) and [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

## License

[MIT](../../LICENSE).
