# sparq-reason-dl

<p>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

**Opt-in OWL 2 Direct-Semantics support** for the [sparq](../../README.md) RDF engine — a
layered, fail-closed Direct-Semantics checker. **Built: L1** (structural **ALCH** model +
fail-closed reverse RDF mapping), **L2** (syntactic EL/QL/RL profile checker), **L3** (the
terminating **ALCH tableau** — consistency + class satisfiability), and **L4** (the
fragment-dispatch checker + entailment-by-refutation, behind the opt-in `dispatch` feature).
It is a **separate crate** — `sparq-core`, `sparq-engine`, and the wasm build carry zero
Direct-Semantics code, deps, or cost by default; DL is engaged only by depending on this crate.

> **Honest scope.** This crate does **not** implement OWL 2 DL. The tableau is sound and
> complete **only for the exact ALCH fragment** (named classes, ⊤/⊥, ⊓/⊔/¬, ∃/∀ over named
> object properties, GCIs, `rdfs:subPropertyOf`, ground ABox) — anything else is refused
> fail-closed as `Unknown(OutOfFragment)` BEFORE reasoning, and deterministic count-budget
> exhaustion is `Unknown(ResourceBudget)`, never a verdict. The L4 dispatch narrows its RL/EL
> verdicts further behind explicit completeness guards (see the `check` module docs for the
> per-branch table). See the design record for the full scope and deferral ledger.

## 🚀 Quickstart

```rust
use sparq_core::Graph;
use sparq_reason_dl::{extract, ExtractError};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
const TTL: &str = r#"
  @prefix : <http://ex/> . @prefix owl: <http://www.w3.org/2002/07/owl#> .
  @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
  :A rdfs:subClassOf :B .
"#;
let (dict, triples) = Graph::parse_to_triples(TTL, "turtle")?;

// Fail-closed: the whole graph maps into the ALCH structural model, or the FIRST
// out-of-fragment / malformed triple is returned as a typed error.
match extract(&dict, &triples) {
    Ok(onto) => assert_eq!(onto.len(), 1),          // one SubClassOf axiom
    Err(ExtractError::OutOfFragment(_)) => {}       // e.g. a cardinality / nominal / inverse
    Err(_) => {}
}
# Ok(()) }
```

## ✨ Features

- **Structural ALCH model** (`model`) — `Axiom` / `ClassExpression` / `ObjectPropertyExpression`
  enums for the fragment: named classes, `owl:Thing`/`owl:Nothing`, `owl:intersectionOf` (⊓),
  `owl:unionOf` (⊔), `owl:complementOf` (¬), `owl:someValuesFrom` (∃R.C), `owl:allValuesFrom`
  (∀R.C) over **named object properties**; GCIs, `owl:equivalentClass`, `owl:disjointWith`,
  `rdfs:subPropertyOf`, `rdfs:domain`/`rdfs:range`, and a **ground ABox**. Purely structural —
  no semantics are attached at L1.
- **Fail-closed reverse RDF mapping** (`extract`) — maps `(Dict, &[[Id; 3]])` into the model per
  the W3C *Mapping to RDF Graphs* tables restricted to ALCH. **A single triple outside the
  fragment aborts the whole extraction** with a typed `ExtractError`, rather than being silently
  dropped: the downstream checker must never reason over a graph it only *partially* understood
  (a dropped axiom can flip a consistency verdict). Understood in full, or refused.
- **Structured rejection taxonomy** — `ExtractError` distinguishes unsupported logical
  constructs, data constructs, malformed lists/expressions, and unclassifiable predicates.
  Every arm has a diagnostic and unit test. Annotations, declarations, and ontology headers
  are recognised and ignored because they carry no ALCH-logical import.
- **Forward RDF renderer** (`render`, bead sq-pbz04.4.7) — `render_to_triples(&Ontology, &mut Dict)`
  maps the structural model back to OWL RDF triples (the inverse of `extract`), enabling
  full-fragment round-trip testing (`RDF → extract → render → extract` ≡ same model) and
  diagnostics via `render_to_turtle`. No extra deps; always compiled.

**Profile checker (L2, bead sq-pbz04.4.2):** `profile::profiles(onto)` checks OWL 2 EL/QL/RL
profile membership via a purely syntactic grammar walk (W3C OWL 2 Profiles §2/§3/§4), returning
a `ProfileSet` of `Membership::In` / `NotIn(reason)` / `Unknown(err)` (the latter only from
`profile::profiles_from_extraction` on an extraction failure). Terminating by construction, no
semantic reasoning.

**ALCH tableau (L3, bead sq-pbz04.4.3):** `tableau::consistency(&Ontology, Budget)` and
`tableau::class_satisfiability(&ClassExpression, &Ontology, Budget)` decide
consistency / class satisfiability for the ALCH fragment via a completion-forest tableau —
GCI internalisation, rules matched modulo the `subPropertyOf` closure, **ancestor subset
blocking** (sufficient precisely because ALCH has no inverse roles), backtracking over
⊔-branches. `tableau::consistency_from_extraction` is the fail-closed RDF-level entry. Verdicts
are tri-state (`Satisfiable` / `Unsatisfiable` / `Unknown`); budgets are deterministic COUNTS
(`max_nodes` / `max_rule_applications` — wall-clock banned). The termination + soundness +
completeness argument (Baader–Sattler 2001) is reproduced in the `tableau` module docs. `nnf`
supplies negation normal form and the finite subexpression closure the argument rests on.

**Fragment dispatch (L4, bead sq-pbz04.4.4, feature `dispatch`):**
`check::DirectChecker::consistency` dispatches an extracted ontology to the first matching
branch — **RL** (`sparq-reason` materialization + clash scan; `Inconsistent` sound via
*checked* Theorem PR1 preconditions, `Consistent` only past a divergence guard over the
constructs implicated in the documented RL rule-set divergences), **EL** (`sparq-reason-el`
classification; verdicts only for a pure ⊤-free EL+⊥ TBox with zero skipped axioms), **QL**
(opt-in `dispatch_ql`, sq-fj8lj: `sparq-reason-ql`'s DL-Lite_R violation-query checker, whose
OWN capture accounting owns the verdict; without it, always `Unknown(QlConsistencyPending)`), or the
**ALCH tableau** (complete for the fragment). A profile branch that ABSTAINS falls through to that tableau (sq-pbz04.4.8) — sound with no new argument, since every L1-extracted
ontology is inside the ALCH fragment by construction; a branch that DECIDED is never preempted, an abstaining tableau returns the branch's own reason unchanged, and the PR1 punning
abstention (ill-posed input, not an incompleteness guard) does not fall through. `check::DirectChecker::entailment` decides premise ⊨ conclusion per conclusion-axiom by sound refutation
encodings on the tableau (GCI / class-assertion / the fresh-class trick, its sq-pbz04.4.9 role-subsumption lift for `SubObjectPropertyOf`, the sq-zfwzq transitivity lift when enabled, + the record's
desugarings); a future unencoded kind abstains. A **blank-node individual in the conclusion is read EXISTENTIALLY** (sq-pbz04.4.13): a tree-shaped anonymous assertion set rolls up into an `∃`-class assertion
decided soundly, and a non-rollable shape (shared / cyclic / nominal / free-root) abstains `ConclusionAnonymousIndividual` — never a skolem-constant `NotEntailed`. A refutation that exhausts the
tableau's deterministic count budget — and ONLY then — is re-asked of the RL/EL branches under their own unchanged guards (sq-pbz04.4.10): strictly abstention-reducing, it can replace
`Unknown(ResourceBudget)` with a definitive verdict but never widen one a guard refused. Every verdict carries its producing `Branch`; every guard fails
closed (`Unknown(reason)`, never a guess). The `dispatch` feature pulls `sparq-reason` +
`sparq-reason-el` as optional deps — **off by default**, so L1–L3 stay dependency-light.

**Transitive roles (opt-in `dl_transitive`, OFF by default, sq-zfwzq):** `owl:TransitiveProperty`
via the tableau's ∀₊-rule, argued in the `tableau` docs §5a. Transitivity-bearing premises may
also supply the already-established role kind for declaration-free conclusion assertions;
unknown conclusion predicates still fail closed. **Deferred** (each rejected, never
mis-mapped): inverses, cardinality, nominals, `sameAs`, datatypes, keys — see the design record.

## 📚 Learn more

- **How-to** — [`skills/inference/SKILL.md`](../../skills/inference/SKILL.md) (Direct-Semantics section).
- **Design** — [`research/owl2-direct-semantics-scoping.md`](../../research/owl2-direct-semantics-scoping.md)
  (the layered fail-closed scope, the ALCH fragment, and the deferral ledger).
- **Contribute** — [`AGENTS.md`](../../AGENTS.md) and [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

## License

[MIT](../../LICENSE).
