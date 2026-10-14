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

**Fragment dispatch (L4, bead sq-pbz04.4.4, feature `dispatch`):** consistency checking
across guarded RL/EL/QL branches and the ALCH tableau, plus entailment-by-refutation.
See the [crate-level rustdoc](https://docs.rs/sparq-reason-dl/latest/sparq_reason_dl/#l4-fragment-dispatch-guards)
for per-branch guards, fallback rules, and entailment details. The optional reasoner
dependencies stay **off by default**, so L1–L3 stay dependency-light.

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
