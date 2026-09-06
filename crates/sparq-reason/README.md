# sparq-reason

<p>
  <a href="https://crates.io/crates/sparq-reason"><img src="https://img.shields.io/crates/v/sparq-reason.svg" alt="crates.io"></a>
  <a href="https://docs.rs/sparq-reason"><img src="https://docs.rs/sparq-reason/badge.svg" alt="docs.rs"></a>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

**Opt-in RDFS / OWL-RL / Notation3 reasoning** for the [sparq](../../README.md) RDF engine.

It forward-chains the deductive closure (RDFS, the OWL 2 RL property/class axioms, or
user-supplied N3 rules — including RDF 1.2 `<< s p o >>` quoted-triple terms in rule
bodies and heads) over dictionary-encoded triples and **materializes** the entailed
facts, so querying stays exactly as fast as before. Reasoning runs over integer ids (joins
on fixed-width keys); the closure can be maintained incrementally under inserts/deletes, and
the non-default `explain` feature answers `why(triple)` with a proof tree. This crate is
**isolated** — depend on it to get reasoning; the core engine and wasm build carry zero cost.

## 🚀 Quickstart

```rust
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use sparq_core::Graph;
use sparq_reason::{materialize, Profile};

// Parse to (Dict, triples), expand in place with every entailed triple, then index.
let (mut dict, mut triples) = Graph::parse_to_triples(turtle, "turtle")?;
let _added = materialize(Profile::Rdfs, &mut dict, &mut triples); // OwlRl includes RDFS
let g = Graph::from_parts(dict, triples);
# let _ = g;
# Ok(()) }
# const turtle: &str = "<http://ex/a> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://ex/b> .";
```

## ✨ Features

- **RDFS entailment** — the non-explosive subset (rdfs2/3/5/7/9/11), materialized in one pass.
- **OWL 2 RL** — property/class axioms over the same fixpoint engine; use `sparq-reason-el`
  for complete class classification.
- **D-entailment** (opt-in `d-entail`) — `Profile::D`: rdfD1 datatype-typing rule under a
  recognized 30-XSD-datatype map (string/normalizedString/token + the
  language/Name/NCName/NMTOKEN pattern-restricted types, boolean, 13 integer types, decimal,
  double, float, date/dateTime/dateTimeStamp, anyURI, hexBinary/base64Binary; plus always-on
  `rdf:langString`) with **correct typed value-space equality** (`"1"^^xsd:integer` =
  `"1.0"^^xsd:decimal`, never f64). Fail-closed: unmapped datatypes/facet-invalid literals
  rejected; `xsd:time`, durations, XML datatypes deferred.
- **Notation3** — user-supplied `{ … } => { … }` rules with EYE-validated builtins. The exact
  `math:` add/subtract/multiply/negate/abs core is the SHARED `sparq-substrate::numeric` `Dec`
  tower, so the N3 chainer and the SPARQL engine never diverge on exact-decimal arithmetic; the
  EYE-specific edges (quotient scale/type rule, divisor-sign remainder, integer-quotient,
  exponentiation, floor/ceiling rendering, `i128` tier) stay in a thin adapter — closures
  byte-identical (`sq-pbz04.5.1`). `reason_n3_pass_all` = EYE `--pass-all`/`-ground` (closure **plus rules**, `sq-xqchl.2`);
  `reason_n3_query` = `--query` (`sq-xqchl.1`): a query rule evaluated over the closure by the chainer's OWN matcher — builtins/formulae/lists work there too.
- **RIF-Core** (opt-in `rif-core`) — W3C RIF **Core** dialect (monotone Horn subset of
  RIF-BLD/PRD) as `rif::Document` over the N3 chainer. Atoms: frame/membership/subclass
  plus numeric/string/list builtins. **Range-restriction safety enforced** (unsafe rules
  rejected). **Body `Equal` resolved at compile time** (substitution/unification, not
  `owl:sameAs` triples); Equal in heads rejected. Distinct ground constants fail-closed
  pending value-space comparator. SPARQL-RIF entailment and larger RIF dialects deferred.
- **Incremental maintenance** — `MaterializedGraph` keeps the closure current under
  inserts/deletes by exact derivation counting; cost scales with the change, not a re-run.
- **Stratified Datalog** (opt-in `datalog`) — a small native rule dialect (RDFox-parity
  track): single or grouped `NOT { atom, atom }` (negation as failure), `AGGREGATE … BIND
  COUNT(DISTINCT ?v)/SUM/MIN/MAX/AVG(?v) AS ?c`, variable predicates, and numeric `FILTER`
  over the shared exact/float/double tower. The **stratification checker** rejects cycles
  through NOT/AGGREGATE and conservatively couples variable predicates to every relation;
  the semi-naive evaluator and incremental maintainer share that invariant. Surfaced by
  `sparq-cli --features datalog` as `--reason datalog:<rules.dlog>`. <!-- [GPT-5.6] sq-a7bmo, [SONNET-4.6] sq-p4zci -->
- **Quoted-triple inference** (opt-in `quoted-triples`) — RDF 1.2 reifier rules for the
  OWL-RL profile: **reif-dtr** destructures `R rdf:reifies <<( s p o )>>` into the classic
  `rdf:subject`/`rdf:predicate`/`rdf:object` view of `R` (so RL rules reason over reifier
  annotations and the recovered components), and **reif-ctr** constructs the reifies
  triple from classic-reification data — restricted to EXISTING triples over leaf
  components so the Herbrand base stays **finite** (termination argument in the `reify`
  module docs). Triple terms stay **opaque**: reification never asserts the referent triple,
  and nothing rewrites inside a triple term; `ReifyMode::DestructureOnly` drops reif-ctr for
  STRICT opacity (batch, or `MaterializedOwlGraph::with_reify_mode`). Off by default — a
  deliberate, non-normative extension; plain `Profile::OwlRl` closures are unchanged.
- **Proof trees** (`explain` feature) — `why(triple)` returns which rule fired from which
  premises, recursively down to asserted facts (a flat, ZK-witness-friendly shape).
- **RIF/XML importer** (opt-in `rif-xml`) — parse the W3C RIF-Core XML presentation
  syntax into a `rif::Document` with Or-split and Exists-flatten desugaring; fail-closed
  taxonomy rejects `Import` directives, non-Core elements, unknown builtins, and
  malformed XML with named error variants. See the `rif_xml` module docs.
- **Shared join kernels** (opt-in `substrate-join`) — the RDFS predicate join (rdfs2/3/7)
  and the rdfs9 type join drive the *same* `sparq-substrate::join` hash-join body the SPARQL
  engine drives, supplying the reasoner's own key projection + budget monomorphically.
  Also covers the OWL-RL semi-naive Δ⋈full adjacency (`prp-fp`, `prp-ifp`, `prp-trp`): a
  persistent `DeltaAdj` (two `DeltaTable`s, `sq-qonbz.2`) replaces the per-round `FxHashMap`
  probes — same closure output, only the join machinery changes.
  Off by default; byte/bundle ratchets unchanged.
- **Shared term total order** (opt-in `substrate-compare`) — `compare::IdTerm` implements the
  substrate's `CompareTerm` for dictionary ids, so ordering entailed solutions
  (`compare::sort_ids` / `compare::compare_ids`) is parity-identical to the SPARQL engine's `ORDER BY`
  total order (pinned byte-for-byte against a real engine query by `tests/compare_parity.rs`).
  Since sq-wjl8i this is a genuine TOTAL order across mixed literal kinds — kind-first rank,
  exact mixed-tier numeric ties, NaN totalised first (see the substrate `compare` docs).
  Monomorphic (no trait object on the sort loop) and purely additive — no materialiser calls
  it, so the entailed closure and its emission order are unchanged. Off by default.
- **Compiled rules** (opt-in `compiled-rules`) — `n3::compiled`: lower N3 rule text ONCE to
  an id-level IR (constants pre-interned into the caller's `Dict`) and run the semi-naive
  fixpoint DIRECTLY over `[Id; 3]` facts on the shared substrate join kernels — no per-call
  text round-trip. Scoped to the access-control subset (`log:notIncludes`/`log:uri`/`log:(not)equalTo`,
  `string:` concat/encodeForUri/scrape/notGreaterThan, plus RDF 1.2 `<< s p o >>` triple terms
  in premises, matched by component-indexed id unpacking); everything else is a loud compile error.
  Closure set-equality vs `reason_n3` is pinned by `tests/compiled_equivalence.rs`. Off by default.

## 📚 Learn more

- **How-to** — [`skills/inference/SKILL.md`](../../skills/inference/SKILL.md) (profiles,
  incremental maintenance, proof trees, CLI seam).
- **API reference** — [docs.rs/sparq-reason](https://docs.rs/sparq-reason).
- **Design** — the inference verdicts in [`research/`](../../research) and
  [`research/ARCHITECTURE.md`](../../research/ARCHITECTURE.md).
- **Performance** — see the [benchmarks dashboard](https://sparq.jeswr.org/dev/bench).
- **Contribute** — [`AGENTS.md`](../../AGENTS.md) and [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

## License

[MIT](../../LICENSE).
