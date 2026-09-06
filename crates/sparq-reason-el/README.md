# sparq-reason-el

<p>
  <a href="https://crates.io/crates/sparq-reason-el"><img src="https://img.shields.io/crates/v/sparq-reason-el.svg" alt="crates.io"></a>
  <a href="https://docs.rs/sparq-reason-el"><img src="https://docs.rs/sparq-reason-el/badge.svg" alt="docs.rs"></a>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

**Opt-in OWL 2 EL classifier** for the [sparq](../../README.md) RDF engine — a
consequence-based reasoner that computes the **complete `rdfs:subClassOf` subsumption
lattice** of an EL ontology, the one thing OWL 2 RL (`sparq-reason`) is **sound but silently
incomplete** for.

RL's rule set is complete only for *assertional* conclusions, so `--reason owl` over a biomedical
EL ontology (GO, ChEBI, SNOMED-style) silently omits derivable subsumptions — e.g. `B ⊑ D` from
`B ⊑ C`, `B ⊑ E`, `C ⊓ E ⊑ D` (RL never composes a TBox conjunction, nor reasons through a fresh
`∃r` successor; Krötzsch, ISWC 2012). EL closes that gap with a deterministic least-fixpoint
saturation (PTIME). A **separate crate** — core engine + wasm carry zero EL code, deps, or cost.

## 🚀 Quickstart

```rust
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use sparq_core::Graph;
use sparq_reason_el::classify_graph;

const TTL: &str = "<http://ex/a> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://ex/b> .";

// Materialize the COMPLETE subsumption lattice as rdfs:subClassOf triples, in place.
let (mut dict, mut triples) = Graph::parse_to_triples(TTL, "turtle")?;
let report = classify_graph(&mut dict, &mut triples); // emits derived subsumptions
let g = Graph::from_parts(dict, triples);             // then query as usual
# let _ = (g, report);
# Ok(()) }
```

For a typed view (super-classes, subsumption test, unsatisfiable classes) use
`Classifier::classify`, which returns a `ClassHierarchy` without mutating the graph.

## ✨ Features

- **Complete EL+⊥ classification** — normalizes the TBox to the four Baader–Brandt–Lutz normal
  forms, then saturates `S(C)` / `R(r)` under completion rules **CR1–CR5** to a fixpoint.
- **CR4 existential traversal** — the load-bearing rule RL lacks; reasons through `∃r` links.
- **Same dict/Graph seam as RL** — emits the lattice as `rdfs:subClassOf` triples queryable by plain BGP eval; no store changes.
- **Unsatisfiability** — named clashes surface in `unsatisfiable_classes`; global `owl:Thing ⊑ owl:Nothing` in `Report::thing_unsatisfiable`.
- **Safe nominals — CR6** — singleton `owl:oneOf` (`{a}`) and object-valued `owl:hasValue`
  (`∃r.{a}`) classify (the reachability-guarded merge rule; every derivation sound, with
  negative tests pinning the guard). Completeness is claimed for typical safe usage, NOT for
  every EL++ nominal interplay; ABox `rdf:type` assertions are not internalized (TBox only).
- **Self-restrictions — CR-Self** — `owl:hasSelf "true"^^xsd:boolean` + `owl:onProperty r` is the
  EL profile's `ObjectHasSelf` (`∃r.Self`, local reflexivity): `X ⊑ ∃r.Self ⇒ (X,X) ∈ R(r)`,
  `∃r.Self ⊑ D` threads via the self-concept atom + CR1, and a SAME-NOMINAL self-link `({a},{a}) ∈ R(r)`
  (an asserted/derived `a r a`) reads off as `∃r.Self ∈ S({a})` (CRs3 — sound: a nominal is a singleton).
  With `rbox` it propagates to `∃s.Self` for `r ⊑* s`; generic `(X,X)` links never trigger either,
  malformed shapes stay counted skips, `abox` self-loops realise as `a r a`.
- **RBox role automaton + lattice readoff** *(opt-in `rbox` feature, Phases E2/E3)* — `rdfs:subPropertyOf`
  inclusions (**CR10**), `owl:propertyChainAxiom` + `owl:TransitiveProperty` compositions (**CR11**), incl. the
  SNOMED-critical right-identity `r ∘ s ⊑ s`; and a **role-lattice readoff**: `classify_graph` also emits the
  NON-REFLEXIVE told-inclusion closure as `rdfs:subPropertyOf` triples (`Report::emitted_role_subsumptions`
  counts the new ones). A told RBox that is NOT regular (a role-dependency cycle through a property-chain constraint — forbidden by the OWL 2 global restrictions) is flagged via
  `Report::rbox_non_regular`: saturation still terminates and stays sound, but classification may be incomplete (honest, never silent). OFF by default: zero role-automaton code without it.
- **Transitive reduction → Hasse diagram** *(opt-in `hasse` feature, Phase E3)* — `DirectHierarchy`
  reduces the full closure to the **direct (immediate) subsumers**, collapses **equivalence
  cliques**, and `classify_hasse_graph` emits the COMPACT taxonomy (direct `rdfs:subClassOf` +
  `owl:equivalentClass`) — O(N) Hasse edges on a deep chain instead of the O(N²) full closure.
- **Concrete domains — CR7–CR9** *(opt-in `cdomain` feature)* — faceted `owl:onDatatype` +
  `owl:withRestrictions` min/max{In,Ex}clusive ranges over `xsd:decimal`/`xsd:integer`(+derived,
  implicit bounds included) and exact-numeric `DataHasValue`/singleton `DataOneOf` points, decided
  EXACTLY on the shared `sparq_substrate::numeric` value tower (never lossy f64): an **empty** range
  clashes via CR5, a **proven** containment threads subsumptions through data-property existentials.
  Anything not exactly decidable is **deferred, never guessed** (matrix in `src/cdomain.rs`).
- **ABox realisation & whole-ontology consistency — CR6 nominals** *(opt-in `abox` feature)* —
  internalizes `ClassAssertion` (`{a} ⊑ C`) and `ObjectPropertyAssertion` (`{a} ⊑ ∃p.{b}`) as
  SAFE-NOMINAL axioms over CR6, then reads off instance typings, `owl:sameAs`, and a whole-ontology
  `inconsistent` verdict (`{a} ⊑ ⊥` or `⊤ ⊑ ⊥`) via the additive `realize` / `realize_graph` entry
  — every emitted fact holds in EVERY model; TBox `Classifier::classify` stays **byte-identical**.
  With `cdomain` too, `a q 5` ⇒ `{a} ⊑ ∃q.{5}` (a CR9 point); unsupported shapes stay counted skips.
- **Keys, negative assertions & differentFrom** *(also `abox`)* — `owl:hasKey` merges two DISTINCT
  named individuals in the key class that share a value on EVERY key property (`owl:sameAs`);
  a PARTIAL key match cannot fire (object keys match a shared nominal successor, data keys a
  shared literal term — sound). An `owl:NegativePropertyAssertion` is a clash iff the positive is
  asserted/derived; `owl:differentFrom` is read off only from a derived nominal clash
  (`{a} ⊓ {b} ⊑ ⊥`) or asserted-inequality symmetry, and a `sameAs`/`differentFrom` coincidence is
  inconsistent. Unsupported key/NPA shapes stay counted skips (fail-closed).
- **Parallel saturation** *(opt-in `par` feature, Phase E4)* — `Classifier::classify_par` /
  `classify_graph_par` run the SAME rules as deterministic bulk-synchronous rounds on a bounded
  `std::thread::scope` pool; the closure is **identical to single-threaded at every thread count**
  (differential + determinism-stress + W3C-EL-corpus CI oracles). Only the COMPUTE phase is parallel; `classify_graph_par_stats` returns the compute/apply attribution (`ParPhaseStats`). Default build stays wasm-safe.
- **Incremental classification under TBox edits** *(opt-in `incremental` feature, Phase E5)* —
  `IncrementalClassifier` keeps the saturation ALIVE across edits and folds a monotone extension
  (added class axioms + brand-new class-expression nodes) into it via a delta-seeded resume. Scoped
  honestly: **any retraction** — and any RBox / concrete-domain / live-node change — takes a full
  re-classification, reported in `IncrementalReport::disposition` (EL saturation keeps no
  derivation provenance, so DRed / ELK affected-context repair is NOT implemented). The post-edit
  hierarchy equals a full re-classification on BOTH paths.
- **Honest fragment reporting** — class axioms outside the active fragment are counted in
  `Report::skipped_axioms`, never silently misapplied. Without `cdomain` that includes ALL
  concrete-domain shapes; with it, only the unsupported remainder above.

**Scope:** EL+⊥ + safe nominals/CR6 + self-restrictions/CR-Self (E1, default), EL+ role reasoning (E2, `rbox`), transitive
reduction (E3, `hasse`), exact-numeric concrete domains (CR7–CR9, `cdomain`), ABox realisation +
whole-ontology consistency (`abox`), parallel saturation (E4, `par` — **single-threaded by
default**), incremental-addition classification (E5, `incremental`). Constructs outside EL entirely
(union / complement / `allValuesFrom` / cardinality / multi-individual `oneOf`) are always skipped.
Enable with `features = ["rbox", "hasse", "cdomain", "abox", "par", "incremental"]`. The
`snomed_go_scale_bench` example (`--features rbox,hasse`) is a *relative* (dimensionless, no hard-coded
ms) end-to-end scaling check confirming normalise + RBox + Hasse compose with no hidden quadratic.

## 📚 Learn more

- **How-to** — [`skills/inference/SKILL.md`](../../skills/inference/SKILL.md) (EL section). **From the CLI** ([OPUS-5] sq-2ch27, Phase E6): `sparq-cli`'s own opt-in `el` feature (which pulls this crate *with* `rbox` but **without** `cdomain`, so that surface is the E1+E2 fragment and concrete-domain axioms stay in `skipped_axioms`) adds `classify <file> <format> [out.nt]` — the classification report as `name<TAB>value` lines on stdout, lattice-augmented graph to `out.nt` — plus the `el` reasoning profile (`query … --reason el`, `reason <f> <fmt> el`). Honest-incompleteness signals (`skipped_axioms`, `rbox_non_regular`, unsatisfiable classes) also print a NOTE on stderr, and without the feature `--reason el` exits 2 naming it rather than silently downgrading to the incomplete `owl` profile. See [`skills/cli/SKILL.md`](../../skills/cli/SKILL.md).
- **API reference** — [docs.rs/sparq-reason-el](https://docs.rs/sparq-reason-el).
- **Design** — [`research/owl2-el-ql-reasoning-spike.md`](../../research/owl2-el-ql-reasoning-spike.md)
  (why EL, the RL-incompleteness proof, the phased plan).
- **Contribute** — [`AGENTS.md`](../../AGENTS.md) and [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

## License

[MIT](../../LICENSE).
