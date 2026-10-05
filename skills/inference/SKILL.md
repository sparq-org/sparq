---
name: inference
description: Use when you need RDFS, OWL 2 RL, or Notation3/EYE-rule entailment over a sparq RDF graph — materialize the deductive closure (forward-chaining), query the entailed triples, maintain the closure incrementally under inserts/deletes, get derivation proof-trees (why()), or check OWL inconsistency; backed by the sparq-reason crate. For the COMPLETE OWL 2 EL class-subsumption lattice (which RL is sound-but-incomplete for), use the separate opt-in sparq-reason-el classifier (CR1-CR5 saturation). For OWL 2 QL certain-answer query rewriting (PerfectRef over DL-Lite_R, EXPERIMENTAL, fail-closed CQ-shape gate), use the separate opt-in sparq-reason-ql crate.
---

# sparq-inference

`sparq-reason` is sparq's **opt-in** reasoning crate: it forward-chains the deductive closure (RDFS, OWL 2 RL, or user-supplied Notation3 rules) over dictionary-encoded triples and **materializes** the entailed facts so querying stays exactly as fast as before. The core engine carries zero reasoning code/cost unless you depend on this crate. Reasoning works at the `[Id;3]` (RDFS/OWL) or `n3::Term` (N3) level; you wire the result back into a `sparq_core::Graph` to query it.

## Quickstart

Add the dep (native targets only — see Gotchas):

```toml
[dependencies]
sparq-core   = "0.1"
sparq-reason = "0.1"   # default features include `parallel`
```

Parse → materialize the RDFS closure in place → build a queryable graph (the canonical seam, exactly what the CLI does):

```rust
use sparq_core::Graph;
use sparq_reason::{materialize, Profile};

// 1. Parse to (Dict, triples) WITHOUT building indexes yet.
let (mut dict, mut triples) = Graph::parse_to_triples(turtle_text, "turtle")?;
let base = triples.len();

// 2. Expand `triples` in place with every entailed triple. Returns NEW triple count.
let added = materialize(Profile::Rdfs, &mut dict, &mut triples);
eprintln!("RDFS: {base} -> {} triples (+{added} entailed)", triples.len());

// 3. Build the indexed graph from the materialized closure and query as usual.
let g = Graph::from_parts(dict, triples);
# Ok::<(), String>(())
```

CLI equivalent (materialize and optionally dump the closure as N-Triples):

```bash
cargo run --release -p sparq-cli -- reason ontology.ttl turtle rdfs            # rdfs | owl | n3
cargo run --release -p sparq-cli -- reason ontology.ttl turtle owl out.nt      # write full closure
cargo run --release -p sparq-cli -- query data.ttl 'SELECT ...' --reason rdfs  # reason then query

# OWL 2 EL classification — the class hierarchy RL cannot reach (opt-in `el` feature). Complete
# for E1+E2 only: the CLI omits `cdomain`, so concrete-domain axioms land in `skipped_axioms`.
cargo run --release -p sparq-cli --features el -- classify ontology.ttl turtle lattice.nt
cargo run --release -p sparq-cli --features el -- query ontology.ttl turtle 'SELECT ...' --reason el
```

## Key APIs

Re-exported from the crate root (`sparq_reason::…`):

```rust
// Which regime to materialize. OwlRl includes RDFS. `D` is opt-in (`d-entail` feature).
pub enum Profile { Rdfs, OwlRl, /* #[cfg(d-entail)] */ D }
impl Profile { pub fn parse(s: &str) -> Option<Profile>; } // "rdfs" | "owl" | "owl-rl" | "d"

// Batch materialization (expand in place; returns NEW triples; idempotent).
pub fn materialize(profile: Profile, dict: &mut Dict, triples: &mut Vec<[Id;3]>) -> usize;
pub fn materialize_rdfs (dict: &mut Dict, triples: &mut Vec<[Id;3]>) -> usize;
pub fn materialize_owl_rl(dict: &mut Dict, triples: &mut Vec<[Id;3]>) -> usize;

// Opt-in `profile` feature: closure growth/time by stable rule group.
pub fn materialize_profiled(profile: Profile, dict: &mut Dict, triples: &mut Vec<[Id;3]>)
    -> (usize, profile::Report);
// Pass `profile::Profiler::with_progress(callback)` to `materialize_profiled_with`
// when progress notifications are required.

// D-entailment (datatype / value-space) — opt-in `d-entail` feature, `sparq_reason::dtype`.
// rdfD1 datatype-typing under a recognized datatype map; CORRECT TYPED value equality
// ("1"^^xsd:integer ≡ "1.0"^^xsd:decimal — canonical decimal, NOT an f64 fast path).
// `materialize(Profile::D, …)` uses the STANDARD map; pass a custom map via:
#[cfg(feature = "d-entail")]
pub fn materialize_d(d: &Recognized, dict: &mut Dict, triples: &mut Vec<[Id;3]>) -> usize;
#[cfg(feature = "d-entail")]
pub fn d_value_eq(lex_a: &str, dt_a: &str, lex_b: &str, dt_b: &str) -> bool; // value-space eq
#[cfg(feature = "d-entail")]
pub struct Recognized; // Recognized::standard() / ::new(iris) / ::default() (string+langString)

// OWL inconsistency check (cax-dw / cls-com / eq-diff / cls-nothing clashes) — run AFTER OWL closure.
pub fn inconsistencies(dict: &Dict, triples: &[[Id;3]]) -> Vec<String>;

// Notation3 / EYE-style forward rules (`{ premise } => { conclusion }` + builtins).
pub fn reason_n3(dict: &mut Dict, src: &str) -> Result<Vec<[Id;3]>, String>;
pub fn reason_n3_proof(dict: &mut Dict, src: &str)
    -> Result<(Vec<[Id;3]>, Vec<ProofStep>), String>;          // EYE --proof analogue
pub fn reason_n3_terms(src: &str, base: Option<&str>) -> Result<N3Closure, String>; // term-level, no Dict
pub fn reason_n3_terms_with_resolver(src, base, resolver: Option<&Resolver>) -> Result<N3Closure, String>;
// EYE --pass-all / --pass-all-ground: the closure PLUS the document's own rules, echoed
// back as ONE N3 document (the chainer consumes rules, so --pass output alone can derive
// nothing further). Closure statements are sorted (deterministic), then rules in document
// order — full <…> IRIs, no @prefix reconstruction, so NOT byte-identical to EYE's writer.
pub fn reason_n3_pass_all(src: &str, vars: RuleVars) -> Result<String, String>;
// EYE --query: every INSTANTIATED conclusion of the query document's forward rules over the
// deductive closure of `data` — a PROJECTION, so a conclusion already in the closure is still
// an answer (unlike --pass-only-new). The premise uses the chainer's own matcher, so builtins,
// quoted `{ … }` formulae and `( … )` lists all work. The query document's FACTS are not loaded
// as data; its `<=` rules ARE available to the premise. No forward rule => error, not "".
pub fn reason_n3_query(dict: &mut Dict, data: &str, query: &str) -> Result<Vec<[Id;3]>, String>;
pub fn reason_n3_query_terms(data: &str, query: &str) -> Result<Vec<[Term;3]>, String>; // term-level
pub enum RuleVars { N3, VarIris }  // `?x` (re-parses as the same rule, so re-running is a
    // fixpoint) | SWAP `var:` IRIs (`--pass-all-ground`: no `?x` survives in a RULE, at any
    // depth — quoted `{ … }` formulae included — but the rule is then constants; the grounded
    // form is for RDF consumers, not re-reasoning)
pub fn reason_n3_stratified(dict: &mut Dict, strata: &[&str])   // stratum-by-stratum closure; carries
    -> Result<StratifiedN3Closure, String>;  // each closure in memory (no re-serialize); the sound
    // driver for the non-monotonic ops (store-scoped log:notIncludes, log:collectAllIn/forAllIn);
    // per-stratum blank scope. Fields: facts (final interned closure), strata_facts (sizes).

// Incremental closure maintenance (closure stays == from-scratch materialize on the current base).
pub struct MaterializedGraph;     // RDFS;     mutations are dictionary-free
impl MaterializedGraph {
    pub fn new(dict: &mut Dict, base: &[[Id;3]]) -> Self;
    pub fn insert(&mut self, triples: &[[Id;3]]) -> usize;     // returns # actually added
    pub fn delete(&mut self, triples: &[[Id;3]]) -> usize;     // exact-count retraction; as cheap as insert
    pub fn contains(&self, t: &[Id;3]) -> bool;
    pub fn closure(&self) -> Vec<[Id;3]>;                      // sorted, deduped
    pub fn iter(&self) -> impl Iterator<Item=[Id;3]> + '_;
    pub fn full_rebuilds(&self) -> usize;                      // TBox edits => full rematerialize
}
pub struct MaterializedOwlGraph;  // OWL 2 RL; insert/delete take &mut Dict (fallback path interns)
impl MaterializedOwlGraph { pub fn new(dict: &mut Dict, base: &[[Id;3]]) -> Self;
    pub fn insert(&mut self, dict: &mut Dict, triples: &[[Id;3]]) -> usize;
    pub fn delete(&mut self, dict: &mut Dict, triples: &[[Id;3]]) -> usize;
    pub fn mode(&self) -> OwlMode; /* + contains/closure/len/full_rebuilds */ }
pub enum OwlMode { CountingMono, CountingFixpoint, Fallback }

pub struct MaterializedN3Graph; // user N3 rules; operates on n3::Term facts
impl MaterializedN3Graph { pub fn new(rules_src: &str, base_facts: &[[Term;3]]) -> Result<Self, String>;
    pub fn insert(&mut self, facts: &[[Term;3]]) -> usize;     // ground facts only
    pub fn delete(&mut self, facts: &[[Term;3]]) -> usize;
    pub fn mode(&self) -> N3Mode;          // Counting (incremental) | Fallback (re-runs engine)
    pub fn fallback_reason(&self) -> Option<&str>;  // None  <=>  counting path active
    pub fn closure(&self) -> Vec<[Term;3]>; /* + contains/len/full_rebuilds */ }
pub enum N3Mode { Counting, Fallback }

// `explain` feature only (see Gotchas): one derivation witness as a flat DAG.
pub fn why(&self, dict: &Dict, t: [Id;3]) -> Option<ProofTree>;          // RDFS / OWL graphs
pub fn why(&self, fact: &[Term;3])        -> Option<ProofTree>;          // N3 graph
pub struct ProofTree;  // .nodes() -> &[ProofNode], .root(), .conclusion(), .to_json(), .to_text()
pub struct ProofNode { pub conclusion: [String;3], pub rule: String, pub premises: Vec<u32> }
pub struct ExplainOpts { pub max_depth: usize, pub max_nodes: usize } // why_with(.., opts)
```

`Id` / `Dict` come from `sparq_core::dict`; `Term`, `Rule`, `N3Closure`, `ProofStep`, `Resolver` from `sparq_reason::n3` (re-exported at the crate root).

```rust
// `compiled-rules` feature only (`sparq_reason::n3::compiled`, sq-zgbso.3): id-level
// COMPILED N3 evaluation for the access-control subset — no per-call text round-trip
// (the rule IR pre-interns its constants into the caller's Dict; the fixpoint joins run
// on the shared sparq-substrate kernels over [Id;3] facts).
pub fn compile(src: &str) -> Result<CompiledRuleSet, String>;   // parse + lower ONCE
pub fn eval(dict: &mut Dict, facts: &[[Id;3]], rules: &CompiledRuleSet) -> Vec<[Id;3]>;
pub fn intern_facts(dict: &mut Dict, src: &str) -> Result<Vec<[Id;3]>, String>; // test/harness fact loader
impl CompiledRuleSet { pub fn bind(&self, dict: &mut Dict) -> BoundRuleSet<'_>; // intern the symbol table
                       pub fn n_rules(&self) -> usize; pub fn n_facts(&self) -> usize; }
impl BoundRuleSet<'_> { pub fn eval(&self, dict: &mut Dict, facts: &[[Id;3]]) -> Vec<[Id;3]>; }
```

Compiled-rules scope is EXACTLY the WAC/ACP/ODRL-spike corpus subset (scoped `log:notIncludes` over stratum-complete predicates, `log:uri`, `log:(not)equalTo`, `string:` concatenation / encodeForUri / scrape / notGreaterThan) **plus RDF 1.2 triple terms in premises** (`sq-6d43t`); anything else is a loud `compile` error — full N3 stays with `reason_n3`. Closure set-equality vs `reason_n3` is pinned by `crates/sparq-reason/tests/compiled_equivalence.rs`.

**Compiled-rules triple terms (`sq-6d43t`).** A GROUND `<< s p o >>` is an ordinary symbol-table constant anywhere (fact, pattern position, builtin argument, conclusion): `bind` interns it through the Dict's content-addressed RDF 1.2 triple-term path, so it resolves to the SAME id a store-loaded `<<( s p o )>>` carries. A quotation that still contains VARIABLES is admitted in PREMISE positions and compiles to a component-indexed unpack step — the enclosing join binds the candidate's triple-term id, then the unpack reads its three component ids straight out of the dictionary record (no term reconstruction, no allocation) and binds first occurrences / filters already-bound variables and constants, left to right, nesting through the OBJECT. Three shapes are deliberately loud `compile` errors instead: a quotation with variables inside a `log:notIncludes` body (the anti-join runs a flat list of plain patterns), a quotation with variables in a CONCLUSION (minting a triple term from bound components can violate RDF 1.2's structural constraints at derivation time, which `eval` has no channel to report), and a nested quotation in SUBJECT position (no dictionary triple term can have a triple-term subject, so such a pattern could only ever fire zero times). Each falls back to `reason_n3`, which handles all three.

## RIF-Core rule front-end (opt-in `rif-core` feature, `sparq_reason::rif`)

The W3C RIF **Core** dialect — the **monotone Horn-rule** common subset of RIF-BLD/PRD — as a `rif::Document` rule front-end *over the same N3 forward chainer* (a faithful in-engine model that `validate`s for safety then lowers to N3; the proven `reason_n3` fixpoint computes the closure). Build with `features = ["rif-core"]`; **OFF by default**, so the lean/wasm build links zero RIF code (no new `Profile` variant — no match-arm churn anywhere).

```rust
# #[cfg(feature = "rif-core")] {
use sparq_reason::rif::{Atom, Builtin, Document, Rule, Term};
use sparq_core::dict::Dict;
// uncle(?n,?u) :- parent(?n,?p) , brother(?p,?u)   — the canonical RIF Core rule (W3C rif01).
let mut doc = Document::new();
doc.push(Rule::fact(Atom::Frame { obj: Term::Iri("ex:Emeka".into()),     pred: Term::Iri("ex:parent".into()),  val: Term::Iri("ex:Oke".into()) }));
doc.push(Rule::fact(Atom::Frame { obj: Term::Iri("ex:Oke".into()),       pred: Term::Iri("ex:brother".into()), val: Term::Iri("ex:Chi".into()) }));
doc.push(Rule::implies(
    vec![Atom::Frame { obj: Term::Var("n".into()), pred: Term::Iri("ex:uncle".into()), val: Term::Var("u".into()) }],
    vec![Atom::Frame { obj: Term::Var("n".into()), pred: Term::Iri("ex:parent".into()),  val: Term::Var("p".into()) },
         Atom::Frame { obj: Term::Var("p".into()), pred: Term::Iri("ex:brother".into()), val: Term::Var("u".into()) }],
));
let mut dict = Dict::new();
let _entailed: Vec<[sparq_core::dict::Id;3]> = doc.closure(&mut dict)?;  // monotone forward-chaining closure
# }
# Ok::<(), Box<dyn std::error::Error>>(())
```

- **Atoms.** `Frame { obj, pred, val }` (`o[p->v]` → triple), `Member { obj, class }` (`o # c` → `rdf:type`), `Subclass { sub, sup }` (`c1 ## c2` → `rdfs:subClassOf`), `Equal { left, right }` (body-only; see Equal-atom semantics below), and `Builtin { op, args }` (body-only).
- **Equal-atom semantics (sq-pbz04.5.4 + sq-26vwp — RIF-Core fidelity; body Equal resolved at compile time, NEVER as an `owl:sameAs` triple):**
  1. **Equal in a rule head is REJECTED** — `validate()` returns `RifError::EqualInConclusion`. This is the RIF-Core syntactic restriction (unlike RIF-BLD, Core does not permit equality in conclusions).
  2. **`t = t` (identical after substitution) is eliminated** — `Equal { left: Term::Iri("x"), right: Term::Iri("x") }` (or `?n=?n`) is trivially true and dropped. It contributes NO bindings for range-restriction — if a head variable is SOLELY bound by a `?x=?x` atom, `validate()` rejects the rule with `UnboundHeadVar` (unifying a variable with itself binds nothing).
  3. **`?x = t` (one side a variable) is SUBSTITUTED** — `t` replaces `?x` throughout head+body at validate/lower time, so `?x` becomes **bound-by-substitution** (`?x # C :- ?x = <a>` collapses to the fact `<a> # C`). **`?x = ?y` (two distinct variables) is UNIFIED** — one name is renamed to the other everywhere, so the join requires the SAME node: same-node reflexivity fires *without* any `owl:sameAs` assertion (fixes V2), and an asserted `owl:sameAs` between DISTINCT nodes never over-derives the equality (fixes V1). Substitution runs to a fixpoint, so chained `?x=?y, ?y=t` collapse both to `t`.
  4. **Distinct GROUND constants — the NUMERIC half is DECIDED (sq-anyad), the rest fail closed.** When BOTH sides are well-formed numeric literals, the atom is decided by the shared substrate comparator `Num::cmp_relational` (sq-v5evr / #1646), across the XSD numeric tiers: value-EQUAL (`"1"^^xsd:integer = "1.0"^^xsd:decimal`) is eliminated like `t = t` so the rule fires, and value-UNEQUAL (`1 = 2`) makes the body unsatisfiable so the rule is **vacuous** — it validates, and derives nothing. The tower is exact only within an `i128` mantissa, so a well-formed `xsd:integer`/`xsd:decimal` beyond that range is classified out of it — such a pair is still DECIDED, not deferred: when both sides are exact-tier lexicals they are compared EXACTLY by `cmp_plain_decimal` string arithmetic (arbitrary precision, no `f64` promotion), so the same >`i128` value written with and without a leading `+` is equal and a pair differing by one below an f64 ulp is unequal. Everything else still returns `RifError::DistinctGroundEqual { left, right }`: the **non-numeric literal-equality half** (`pred:boolean-equal`, `pred:literal-not-identical` over strings/booleans/dates), an IRI or `List` operand (including a distinct ground *created* by substitution, e.g. `?x=<a>, ?y=<b>, ?x=?y`), an ill-formed numeric lexical — including one outside the value space of its declared **derived integer datatype**, so `"-1"^^xsd:positiveInteger` and `"128"^^xsd:byte` are refused rather than decided (the sign/bounds facets are checked at arbitrary precision, ahead of both the tower and the exact-lexical path) — a `NaN` operand (a comparator type error, not a verdict), and an out-of-tower exact-tier value paired with an `xsd:float`/`xsd:double` operand (which would need the float's exact decimal expansion). That half stays deferred — the front-end refuses rather than answering incorrectly. Vacuity never excuses an unsafe rule: range-restriction is checked even for an unsatisfiable body. No body `Equal` atom ever reaches N3 lowering — a stray one fails closed, never emits `owl:sameAs`.
- **Builtins (`rif::Builtin`).** Numeric predicates (`NumericEqual`/`LessThan`/`GreaterThan`/`NotLessThan`/`NotGreaterThan`/`NumericNotEqual`) + functions (`NumericAdd`/`Subtract`/`Multiply`/`Divide`), string predicates (`StringContains`/`StartsWith`/`EndsWith`) + functions (`StringConcat`/`StringLength`/`StringUpperCase`/`StringLowerCase`/`StringEncodeForUri`), and list `ListContains`/`ListLength`/`ListConcatenate` (variadic — `is_variadic()` returns `true`; `arity()` is the minimum). `is_filter()` distinguishes a predicate (all args inputs) from a function (last arg = computed output). Each lowers to the equivalent `math:`/`string:`/`list:` N3 builtin. A **deferral ledger** (`rif::UNIMPLEMENTED`) records builtins that are NOT mapped because no sound N3 target exists today (e.g. `func:numeric-integer-divide` truncation semantics, `pred:matches` XSD-regex vs Rust-regex dialect gap, date/time builtins lacking a temporal tower); those entries are tracked, never silently dropped.
- **Builtin SAFETY / range-restriction (enforced).** `Document::validate()` (also called by `to_n3_source`/`closure`) **rejects** an unsafe rule with a `RifError` rather than letting the chainer loop or over-derive: a head variable not bound by a positive body atom (`UnboundHeadVar`), a builtin *input* not range-restricted (`UnboundBuiltinInput`), a builtin in a head (`BuiltinInHead`), wrong arity (`BadBuiltinArity`), Equal in a head (`EqualInConclusion`), or distinct ground constants in a body Equal that numeric value-space equality cannot decide (`DistinctGroundEqual`).
- **MONOTONE — NAF is EXCLUDED by design.** RIF-Core is monotone Horn; negation-as-failure / RIF-PRD actions / aggregation are **not in the dialect** and are not representable in the `Atom` model. Adding facts only ever *adds* conclusions. Larger-RIF surface (RIF-BLD function symbols, the SPARQL-RIF Core Entailment Regime) is documented out-of-scope in `rif::UNIMPLEMENTED` — tracked, never faked. The expressivity ratchet is `sparq-conformance`'s `rif_core_suite` (opt-in `rif-core` feature; `RIF_CORE_FLOOR`, a sparq-EXTENSION row in the central scoreboard).
- **RIF/XML importer** (`rif-xml` feature, `rif_xml::import()` / `rif_xml::import_with_closure()`): parses the W3C RIF-Core XML
  presentation syntax into a `rif::Document`. Applies three sound desugarings at import: body
  `Or` → rule-splitting (Lloyd-Topor, one rule per disjunct); body `Exists` → existential
  vars become ordinary body vars (range-restriction validated by `Document::validate`);
  multi-slot `Frame` → per-slot conjunction (see below, sq-jsgyn). Fail-closed:
  `Import` directives, non-Core elements, unknown `External` IRIs, named-argument uniterms,
  and malformed XML each produce a named `ImportError` variant. Parsing only — no new inference
  beyond the existing `rif-core` forward chainer. Unblocks sq-pbz04.5.5 (W3C RIF WG test-suite arm).
  **Positional predicate atoms** (`<Atom><op>P</op><args>…</args></Atom>`, sq-n7y15): the dominant
  form in real W3C RIF Core test files. Arity-1 maps to `Atom::Member` (membership `a # C`),
  arity-2 maps to `Atom::Frame` (frame atom `a[P → b]`). Arity-0 and arity-3+ are rejected
  fail-closed (`ImportError::UnrecognizedElement`) — no sound mapping exists in the Core model.
  **Multi-slot Frame desugaring** (`obj[p1->v1 p2->v2 …]`, sq-jsgyn): a `<Frame>` with N `<slot>`
  children desugars into N `Atom::Frame` atoms — one per `(pi, vi)` pair, sharing the same `obj`.
  Under RIF-Core §2.3 a multi-slot frame is the conjunction of per-slot frames; in body position
  this becomes a body `And`, in head position N head atoms are added, and as a bare fact N
  `Rule::fact` entries are produced. Fail-closed: zero slots → `MalformedXml`; named-arg slot →
  `NamedArgUniterm`; duplicate `<object>` → `MalformedXml`. `<slot>` is multi-cardinality;
  `<object>` remains single-cardinality.
  **Imports-closure consistency check** (`import_with_closure(xml_bytes, resolver)`, sq-wbql1):
  unlike `import()` which blanket-refuses any `<Import>` directive, `import_with_closure` accepts
  a caller-supplied `resolver: impl Fn(&str) -> Option<Vec<u8>>` and performs a GENUINE consistency
  check: (1) profile-checks the `<Import>` `profile` attribute — a non-Core profile IRI (BLD, PRD,
  OWL-Direct, …) → `ImportError::InconsistentImport` (a NON-VACUOUS detection, distinct from a
  blanket refusal); (2) if the resolver returns bytes for the import location, the imported document
  is parsed as RIF-Core and its rules are merged; the combined rule set is then validated — a
  validation failure → `ImportError::InconsistentImport`; (3) if the resolver returns `None`, the
  import is still rejected fail-closed with `ImportError::ImportDirective`. The fail-closed
  invariant: an inconsistent/unresolvable/incompatible import is ALWAYS rejected; a consistent
  import (resolvable, Core-compatible profile, combined rules pass `validate()`) is accepted and
  the merged `Document` returned. The W3C RIF ImportRejectionTests target profile-mismatch
  invalidity; with a file-system resolver wired to the fetched `Core_v1.22` archive, tests using
  non-Core `profile` attributes graduate from `skip:imports` to `Outcome::Pass`.

## Stratified Datalog rules (opt-in `datalog` feature, `sparq_reason::datalog`)

RDFox-parity track (`research/stratified-datalog-rules.md`): a small native
rule dialect with single/grouped `NOT` (negation as failure),
`AGGREGATE COUNT`/`SUM`/`MIN`/`MAX`/`AVG` atoms (including
`COUNT(DISTINCT ?v)`), variable predicates, and numeric `FILTER` over exact,
float, and double values. Its **stratification checker** rejects programs with a
recursion cycle through NOT/AGGREGATE; dependencies are class-granular for
`rdf:type`, while variable predicates conservatively couple to every relation. It has a
semi-naive per-stratum evaluator on the shared substrate join kernels, and
**incrementally maintained materialization** (`MaterializedProgram`: DRed for positive
strata, stratum-boundary rederivation for `NOT`/`AGGREGATE` strata).

```rust
use sparq_reason::datalog::{eval, parse_program, stratify, MaterializedProgram};

pub fn parse_program(dict: &mut Dict, src: &str) -> Result<Program, String>;
pub fn stratify(dict: &Dict, p: &Program) -> Result<Stratification, String>; // checker alone
pub fn eval(dict: &mut Dict, facts: &[[Id;3]], p: &Program) -> Result<Vec<[Id;3]>, String>;
impl Program { pub fn n_rules(&self) -> usize; }
impl Stratification { pub fn n_strata(&self) -> usize; }

// Incrementally maintained materialization (sq-4foq0). insert/delete return the
// exact closure delta; delete of a still-derivable fact keeps it (owner changes);
// update() is one batch: new base = (base \ deletes) ∪ inserts.
impl MaterializedProgram {
    pub fn new(dict: &mut Dict, facts: &[[Id;3]], p: Program) -> Result<Self, String>;
    pub fn insert(&mut self, dict: &mut Dict, facts: &[[Id;3]]) -> usize;
    pub fn delete(&mut self, dict: &mut Dict, facts: &[[Id;3]]) -> usize;
    pub fn update(&mut self, dict: &mut Dict, ins: &[[Id;3]], del: &[[Id;3]]) -> (usize, usize);
    pub fn contains(&self, f: &[Id;3]) -> bool;
    pub fn closure(&self) -> Vec<[Id;3]>;   // a SET; also len() / is_empty()
}
```

```rust
let mut dict = Dict::new();
let rules = parse_program(&mut dict, r#"@prefix ex: <http://ex/> .
[?x, ex:deg, ?c] :- AGGREGATE([?x, ex:edge, ?y], [?y, ex:tag, ?t]
                               ON ?x BIND COUNT(DISTINCT ?y) AS ?c) .
[?x, a, ex:Hub]  :- [?x, ex:deg, ?c], FILTER(?c >= 3) .
[?x, a, ex:Leaf] :- [?x, a, ex:Node],
                     NOT { [?x, a, ex:Hub], [?x, ex:disabled, "yes"] } ."#)?;
let closure = eval(&mut dict, &facts, &rules)?; // inputs + derivations, a SET
```

Grouped absence accepts `NOT { atom, atom }` or `NOT EXISTS { atom, atom }`; atoms
may be comma- or period-separated. The whole conjunction is tested jointly, with
free variables existential and group-local. Legacy `NOT atom` is unchanged. Variable
predicate atoms scan all relations; a variable head predicate emits only when its
binding is an IRI. Aggregate numeric inputs use the shared XSD numeric tower and
non-numeric rows fail closed; `FILTER` uses relational numeric comparison, so NaN
also fails the row (including `!=`). Head/FILTER vars must be bound positively;
non-`ON` aggregate-body vars are aggregate-local (name collisions rejected). `COUNT(?v)`
counts distinct full-body matches per group; `COUNT(DISTINCT ?v)` de-duplicates the
projected value within each group. Counts mint `xsd:integer` literals. `SUM`
and `AVG` follow SPARQL numeric promotion (`AVG` of integers is `xsd:decimal`), while
`MIN`/`MAX` preserve the original extremal term id. `xsd:float`/`xsd:double` operands
are accepted (not rejected) on all four, so `SUM`/`AVG` can emit float/double; because
float addition is not associative the fold order is PINNED to ascending value, ties
broken by RDF-term CONTENT (datatype, then lexical form — not by dictionary id, which
follows interning order), and `MIN`/`MAX` order `NaN` below `-INF` (`NaN` is not a row
failure here, unlike `FILTER`) with `+0.0`/`-0.0` a content-broken tie — so the closure
is a function of the completed lower strata, not of derivation order. Semi-naive rounds run per stratum.
Incremental maintenance is differential-pinned (closure == from-scratch `eval` after
every randomized insert/delete step) and skips strata whose input predicates did not
change; its per-update set/index bookkeeping is O(affected visible input) — the
incrementality win is delta-driven RULE-FIRING work (deterministic counters), not set
ops. <!-- [GPT-5.6] sq-citho / sq-a7bmo --> <!-- [FABLE-5] sq-4foq0 -->

**CLI surface** (`sparq-cli --features datalog`; [SONNET-4.6] sq-p4zci). The rules live in a file,
because unlike `rdfs`/`owl` a Datalog program is user-supplied, so the reasoning profile carries an
argument — `datalog:<rules.dlog>`, split on the first `:`:

```bash
# Materialize the closure to N-Triples...
sparq-cli reason graph.nt ntriples datalog:rules.dlog closure.nt
# ...or reason-then-query in one shot; derived facts are ordinary triples, so this is plain BGP
sparq-cli query graph.nt ntriples 'SELECT ?x WHERE { ?x a <http://ex/Leaf> }' \
  --reason datalog:rules.dlog
```

Reasoning runs at the parse → index-build seam (as `--reason rdfs` does), and stderr reports the
program shape and the expansion: `reasoned [datalog rules.dlog]: <n> rule(s) in <k> stratum/strata;
<base> -> <total> distinct triples (+<derived> derived) in <elapsed>s`. A rule document outside the
documented fragment — or with a plain syntax error — (exit 1, the parser names the construct) and a
program whose recursion cycles through `NOT`/`AGGREGATE` (exit 1, the checker names a predicate on
the cycle) both fail LOUDLY, each message carrying the rules-file path.
Without the feature, `--reason datalog:…` is exit 2 naming it, with **no** fall-back profile —
RDFS/OWL-RL are monotone, so silently substituting one would drop `NOT`/`AGGREGATE` and change the
answer set. The one-shot CLI path uses `eval`; for a long-lived closure under mutation, use
`MaterializedProgram` from the library API above.

## D-entailment datatype typing (opt-in `d-entail` feature, `sparq_reason::dtype`)

The RDF 1.1 Semantics D-entailment regime materializes the **rdfD1 datatype-typing rule**: a well-formed literal of a recognized datatype `d` entails a typing triple. `materialize(Profile::D, …)` adds the recognized 30-XSD-datatype map via `Recognized::standard()` — the `DTYPE_TABLE` single source of truth in `dtype.rs` — plus the always-recognized `rdf:langString` (or bring a custom map via `materialize_d(d, …, …)`):

**Supported datatypes** (complete signed/unsigned integer family, exact decimals/temporal):
- String family: `xsd:string`, `xsd:normalizedString`, `xsd:token`; pattern-restricted derived types `xsd:language`, `xsd:Name`, `xsd:NCName`, `xsd:NMTOKEN`.
- Boolean: `xsd:boolean`.
- Integer family (13 XSD types — `xsd:integer` + 12 derived): `xsd:long`/`xsd:int`/`xsd:short`/`xsd:byte` (signed); `xsd:unsignedLong`/`xsd:unsignedInt`/`xsd:unsignedShort`/`xsd:unsignedByte` (unsigned); `xsd:nonNegativeInteger`/`xsd:positiveInteger`/`xsd:nonPositiveInteger`/`xsd:negativeInteger` (restricted).
- Numeric: `xsd:decimal` (exact, unbounded magnitude via canonical-decimal STRING comparison, never f64), `xsd:double`, `xsd:float` (IEEE 754, distinct value spaces).
- Temporal: `xsd:dateTime`, `xsd:dateTimeStamp`, `xsd:date`.
- URI: `xsd:anyURI`.
- Binary: `xsd:hexBinary`, `xsd:base64Binary` (shared octet-sequence value space).

**Value-space equality (the load-bearing invariant):** `"1"^^xsd:integer` and `"1.0"^^xsd:decimal` denote THE SAME value and must compare equal in D-entailment. Integer/decimal are compared as a canonical-decimal STRING (sign + minimal integer/fraction digits), NEVER via f64 (which aliases integers past 2^53 and loses decimal precision — silent bugs in semantic equality). `xsd:float`/`xsd:double` are IEEE value spaces; `xsd:date` and `xsd:dateTime` are disjoint temporal families (even at the same instant).

**Fail-closed posture:** unmapped datatypes are not typed; facet-invalid literals (`"200"^^xsd:byte`, `" a"^^xsd:token`) are rejected before value mapping; `xsd:time`, duration types, and XML datatypes (`rdf:XMLLiteral`) are deferred — tracked in the design record (`research/d-entailment-datatype-map.md` §3.2), never silently mapped. The `Recognized::default()` set carries ONLY the always-recognized `xsd:string` / `rdf:langString` pair — safe to materialize over arbitrary data.

## Quoted-triple (RDF 1.2 reifier) inference (opt-in `quoted-triples` feature) <!-- [Kern] kern/quoted-triple-infer -->

The loaders desugar every RDF 1.2 quotation form (`<< :s :p :o >>`, `:s :p :o ~ :r`, `{| … |}` annotation blocks) to a reifier node `R` with `R rdf:reifies <<( s p o )>>`, the triple term being ONE opaque structural dictionary id. With `features = ["quoted-triples"]`, `materialize(Profile::OwlRl, …)` additionally runs the two **bridge rules** between that shape and the classic reification vocabulary, alternated with the RL closure to their joint fixpoint:

- **reif-dtr (destructure):** `(r rdf:reifies <<( s p o )>>)` ⊢ `(r rdf:type rdf:Statement)`, `(r rdf:subject s)`, `(r rdf:predicate p)`, `(r rdf:object o)` — so domain/range/subPropertyOf/sameAs reasoning reaches the recovered components.
- **reif-ctr (construct):** `(r rdf:subject s)`, `(r rdf:predicate p)`, `(r rdf:object o)` **and `(s p o)` present in the closure** ⊢ `(r rdf:reifies <<( s p o )>>)`, minting the triple term. Two finiteness guards: only EXISTING triples are ever reified (asserted or derived — never invented), and no component of a constructed triple term may itself be a triple term (quotation-of-a-quotation is never constructed), which keeps the Herbrand base FINITE, materialization terminating AND idempotent (termination argument in `sparq-reason/src/reify.rs`).

**Opacity:** quotation never asserts — `:r rdf:reifies <<( :s :p :o )>>` does NOT entail `:s :p :o` — and no rule rewrites inside a triple term (`owl:sameAs` substitutes whole ids only). Reifier ANNOTATIONS are ordinary triples and get full RL reasoning without touching the quoted content.

**Strict opacity (`ReifyMode`, second increment):** <!-- [FABLE-5] sq-afun3 --> the bridge COMPOSITION (destructure → eq-rep on the classic vocabulary → construct) can still quote an `owl:sameAs`-VARIANT spelling of an existing triple. `materialize_owl_rl_reify(&mut dict, &mut triples, ReifyMode::DestructureOnly)` suppresses that: reif-ctr never runs, so inference never mints a triple term at all — destructure, annotation reasoning, and the RL core are unchanged. `materialize_owl_rl` ≡ `ReifyMode::Bridge` (the full bridge).

**Strict opacity incrementally (third increment):** <!-- [OPUS-5] sq-afun3 --> `MaterializedOwlGraph::with_reify_mode(&mut dict, &base, ReifyMode::DestructureOnly)` fixes the mode at construction; every Fallback re-materialization (the initial one and every mutation's) runs it, so the handle's closure always equals the MATCHING batch oracle — `materialize_owl_rl_reify(dict, base, mode)` from scratch — and inference never mints a triple term under an edit sequence either. `MaterializedOwlGraph::new` ≡ `ReifyMode::Bridge`; `reify_mode()` reports the mode. Mode DETECTION is unchanged (reify vocabulary → Fallback in both reify modes: the counting modes model neither bridge rule).

**OFF by default** (the bridge is a deliberate, non-normative entailment extension): plain `Profile::OwlRl` closures are byte-identical without the feature, and occurrence-guarded even with it (reify-free data pays nothing). `MaterializedOwlGraph` routes reify-vocabulary bases to its documented Fallback mode (incremental == from-scratch parity preserved). No new `Profile` variant, no new deps.

## OWL 2 EL classification (`sparq-reason-el`, separate opt-in crate)

OWL 2 RL is **sound but silently incomplete for class classification**: it has no rule that reasons *through* an existential successor, so `--reason owl` over an EL ontology (GO/ChEBI/SNOMED-style) returns a `rdfs:subClassOf` hierarchy that **silently omits** subsumptions like `A ⊑ D` from `A ⊑ ∃r.B`, `B ⊑ C`, `∃r.C ⊑ D` (Krötzsch, ISWC 2012). **`sparq-reason-el`** closes that gap — a consequence-based classifier that normalizes the TBox (Baader–Brandt–Lutz forms) and saturates `S(C)`/`R(r)` under completion rules **CR1–CR5** to compute the **complete** subsumption lattice, then emits it into the **same** `(Dict, Vec<[Id;3]>)` seam as the RL `scm-*` rules (queryable by plain BGP eval).

```rust,ignore
// Cargo.toml:  sparq-reason-el = "0.1"     // a SEPARATE crate; depending on it is the opt-in
use sparq_core::Graph;
use sparq_reason_el::{classify_graph, Classifier};

// Materialize the complete subsumption lattice in place, then query as usual.
let (mut dict, mut triples) = Graph::parse_to_triples(ttl, "turtle")?;
let report = classify_graph(&mut dict, &mut triples);   // adds derived rdfs:subClassOf edges
let g = Graph::from_parts(dict, triples);

// Or a typed, non-mutating view (super-classes / subsumption test / unsatisfiable classes):
let (dict, triples) = Graph::parse_to_triples(ttl, "turtle")?;
let h = Classifier::classify(&dict, &triples);
let _ = h.super_classes(some_class_id);
let _ = h.unsatisfiable_classes();      // classes forced ⊑ owl:Nothing (e.g. via disjointWith)
let _ = h.report().thing_unsatisfiable; // global owl:Thing ⊑ owl:Nothing clash
```

- **Scope (default, Phase E1):** `EL+⊥` minus RBox — `rdfs:subClassOf`/`owl:equivalentClass`, `owl:intersectionOf`, `owl:someValuesFrom` restrictions, `owl:disjointWith`, `owl:Thing`/`owl:Nothing` — **plus safe nominals** (bead `sq-pbz04.2.1`): a singleton `owl:oneOf` (`{a}`) and an object-valued `owl:hasValue` (`∃r.{a}`) are basic concepts reasoned over by completion rule **CR6** (the reachability-guarded nominal merge, "Pushing the EL Envelope" IJCAI-05 — the guard is what keeps `C ⊑ {a}, D ⊑ {a} ⊬ C ⊑ D` when `D` may be empty; negative tests in `tests/nominals.rs` pin it). Every CR6 derivation is sound; completeness is claimed for the typical safe usage, NOT for every EL++ nominal interplay (unrestricted nominal interaction needs a stronger calculus — ELK line of work, KR 2012). **Global bottom:** `Report::thing_unsatisfiable` is `true` when saturation derives `owl:Thing rdfs:subClassOf owl:Nothing`; `unsatisfiable_classes()` remains a named-class-only list and deliberately excludes `owl:Thing`. **Self-restrictions** (bead `sq-pbz04.2.6`): `owl:hasSelf "true"^^xsd:boolean` + `owl:onProperty r` is the profile's `ObjectHasSelf` (`∃r.Self`, local reflexivity) — decoded as a self-concept atom and reasoned over by **CR-Self-1** (`X ⊑ ∃r.Self ⇒ (X,X) ∈ R(r)`), **CR-Self-2** (`∃r.Self ⊑ D ⇒ X ⊑ D`, realised via the self-concept atom + CR1), and — bead `sq-8zqwb`, EL wave 2 — **CRs3, nominal reflexivity** (a SAME-NOMINAL self-link `({a},{a}) ∈ R(r)`, e.g. an asserted or derived `a r a`, reads off as `∃r.Self ∈ S({a})` — sound because a nominal denotes a singleton, so its r-successor inside itself IS itself). Load-bearing side-condition: a GENERAL `(X,X)` link from `X ⊑ ∃r.X` (CR3) — whose invariant is only `X ⊑ ∃r.X`, NOT `X ⊑ ∃r.Self` — must NEVER trigger CR-Self-2 or CRs3 (only a NOMINAL's self-link is local reflexivity); a malformed `owl:hasSelf` (non-`true`/non-boolean object, missing `owl:onProperty`) stays a counted skip (fail-closed). The default TBox classifier does NOT internalize ABox `rdf:type`/property assertions (that stays its contract in every feature state — see the opt-in `abox` feature below for assertion realisation + whole-ontology consistency). Class axioms outside the recognised fragment are **not applied** and counted in `Report::skipped_axioms` (honest, never silently misapplied). Single-threaded by default (see the opt-in `par` feature below).
- **RBox role reasoning + lattice readoff (opt-in `rbox` feature, Phases E2/E3, beads `sq-xetf7`/`sq-pbz04.2.7`):** add `features = ["rbox"]`. Applies `rdfs:subPropertyOf` role inclusions (**CR10**) and `owl:propertyChainAxiom` + `owl:TransitiveProperty` compositions (**CR11**, incl. the SNOMED-critical right-identity `r ∘ s ⊑ s`) via a saturated role automaton, so links propagate up the role hierarchy and along chains before CR4/CR5 fire. `classify_graph` ALSO emits the **NON-REFLEXIVE told-inclusion closure** as `rdfs:subPropertyOf` triples (readoff of the already-computed `RoleBox::super_of` table — no new saturation; self-pairs excluded). `Report::emitted_role_subsumptions` counts the new triples added (told pairs already in the graph are not re-emitted; second call idempotent). **Regularity honesty (bead `sq-oj06v`):** the OWL 2 global restrictions require the RBox to be REGULAR (a strict order on roles must admit every chain axiom); a TOLD RBox that is not — its role-inclusion/chain dependency graph has a cycle through a property-chain constraint (checked on the told n-ary chains, NOT the binarized forms, so a told left identity `s ∘ p ∘ q ⊑ s` is correctly regular) — sets **`Report::rbox_non_regular`**: CR10/CR11 saturation still terminates and every derived subsumption stays sound, but the EL+ completeness argument assumes regularity, so classification **may be incomplete** (the `skipped_axioms` honesty posture — flagged, never silently wrong; pure inclusion cycles i.e. equivalent roles and binary transitivity `r ∘ r ⊑ r` are NOT flagged). **OFF by default** — zero role-automaton code in the default/wasm build; without it RBox axioms are left unapplied (roles compared for equality only). `Classifier::classify` (typed API) is unchanged.
- **Transitive reduction → Hasse diagram (opt-in `hasse` feature, Phase E3, bead `sq-s2nob`):** add `features = ["hasse"]`. `DirectHierarchy::from_closure(&h)` reduces the *full* closure to **direct (immediate) subsumers** and collapses **equivalence cliques** (`direct_super_classes` / `representative` / `equivalent_classes`); `classify_hasse_graph(&mut dict, &mut triples)` materializes the COMPACT taxonomy — direct `rdfs:subClassOf` + `owl:equivalentClass` edges, **O(N)** on a deep chain instead of the O(N²) full closure `classify_graph` emits. The closure of the direct edges (chased through cliques) re-derives the complete relation, so it loses nothing. **OFF by default** — zero reduction code without it; the full-closure `Classifier`/`classify_graph` API is unchanged. Deterministic (rep = min dict id, sorted output) so the Hasse **edge count** is a hard assertion target; timings advisory.
- **Concrete domains — CR7–CR9 (opt-in `cdomain` feature, bead `sq-pbz04.2.2`):** add `features = ["cdomain"]`. Faceted datatype restrictions — `owl:onDatatype` + `owl:withRestrictions` with `xsd:min/maxInclusive`/`xsd:min/maxExclusive` facets — are decided EXACTLY on the shared `sparq_substrate::numeric` value tower (`Dec` i128 fixed-point, never lossy f64) for `xsd:decimal`, `xsd:integer` and the 12 derived integer types (implicit bounds folded in, so `xsd:byte` + `minInclusive 1000` is genuinely empty; exclusive integer bounds TIGHTEN, so integer `(5, 6)` is empty while decimal `(5, 6)` is not). An **EMPTY** range is `⊑ owl:Nothing` (the clash reaches classes with an `∃p.range` obligation via CR5 → `unsatisfiable_classes()`); a **proven value-space containment** (`[5,10] ⊆ [0,20]`, integer-inside-decimal, point-in-range) threads subsumptions through data-property existentials via the ordinary CR1/CR3/CR4. Exact-numeric `DataHasValue` (`owl:hasValue 5`) and singleton `DataOneOf` (`owl:oneOf (5)`) are point ranges (`{5}`, `{5.0}` and faceted `[5,5]` unify on ONE concept). **Deferred — no verdict is EVER guessed** (stays in `skipped_axioms`): pattern/length/digit facets (an unknown facet defers the WHOLE range — ignoring it could fabricate a containment), float/double or non-numeric bases and bound values, `owl:onDataRange` (cardinality vocabulary, outside EL), `owl:datatypeComplementOf`, ill-formed bounds (`"300"^^xsd:byte`), and mixed range/class-expression nodes. Known sound incompleteness: a decimal-sorted range is not derived ⊆ an integer-sorted one (non-point cases), and a plain facet-free datatype IRI filler keeps its opaque-class treatment. **OFF by default** — zero concrete-domain code and no `sparq-substrate` dep without it; every concrete-domain occurrence is then skipped as before. `tests/cdomain.rs` pins the sat/unsat/deferral matrix with exact-closure oracles. Composed with `abox` (bead `sq-vkq9u`) the same point machinery rescues `DataPropertyAssertion`s — see the ABox bullet below.
- **ABox realisation + whole-ontology consistency (opt-in `abox` feature, bead `sq-pbz04.2.5`):** add `features = ["abox"]`. The additive `realize(&dict, &triples) -> Realization` / `realize_graph(&mut dict, &mut triples) -> AboxReport` entry internalizes `ClassAssertion` (`a rdf:type C` ⇒ `{a} ⊑ C`) and `ObjectPropertyAssertion` (`a p b` ⇒ `{a} ⊑ ∃p.{b}`) as SAFE-NOMINAL axioms over the CR6 machinery, then reads the saturation off: `{a} ⊑ C` (C a NAMED class, incl. `owl:Thing`) ⇒ **`a rdf:type C`**; a derived `{a} ⊑ {b}` ⇒ **`a owl:sameAs b`**; a derived `∃r.Self ∈ S({a})` (bead `sq-pbz04.2.6`, `owl:hasSelf`) ⇒ the property assertion **`a r a`** via `Realization::self_assertions()` (the WG `New-Feature-SelfRestriction-001` "Peter likes Peter"; the CONVERSE — an asserted `a r a` ⇒ `Peter ∈ ∃likes.Self` typings via CRs3, the `New-Feature-SelfRestriction-002` converse shape — graduated in bead `sq-8zqwb`); `{a} ⊑ ⊥` (two disjoint `ClassAssertion`s, an instance of `∃owl:bottomObjectProperty.⊤` or `∃op.owl:Nothing`) or a global `⊤ ⊑ ⊥` ⇒ a whole-ontology **`is_inconsistent()`** verdict (on `Realization` and the returned `ClassHierarchy`). **Every emitted typing/sameAs/inconsistency holds in EVERY model** (soundness over completeness; the CR6 reachability side-condition is untouched — this path only READS the saturation `classify::saturate` produced). The TBox `Classifier::classify`/`classify_graph` are **byte-identical in every feature state** (they NEVER internalize assertions — so the `el-suite` conformance floor is unaffected even under `--all-features`). **Composed with `cdomain` (bead `sq-vkq9u`)** a `DataPropertyAssertion` whose literal lies in the exact numeric tier is ALSO internalized — `a q 5` ⇒ `{a} ⊑ ∃q.{5}`, over the very point range CR9 mints for `DataHasValue 5` — so CR8 containment threads an **asserted VALUE** into the TBox's data-range obligations (`:alice :age 42` with `∃age.[18,∞) ⊑ :Adult` ⊨ `:alice a :Adult`; two asserted values in provably-disjoint ranges ⇒ `is_inconsistent()`). Sound because `a q v` asserts `(a^I, v) ∈ q^I` and `v ∈ {v}^D`; `tests/abox_cdomain.rs` pins it (that feature CONJUNCTION is its own feature-matrix leg). **Fail-closed:** a data-property literal with no minted point range — EVERY literal without `cdomain`, and with it the string / lang-tagged / float-tier / ill-formed-for-its-own-datatype cases — and a non-EL class expression in a `ClassAssertion` stay counted in `Report::skipped_assertions`, never guessed; an INCONSISTENT ontology realises to NOTHING (the verdict is the surface, not an everything-entailed flood). **OFF by default** — zero assertion-reasoning code without it. `src/abox.rs` pins the readoff over the WG `WebOnt-Ontology-001` / `DisjointClasses-002` / `New-Feature-BottomObjectProperty-001` / `WebOnt-Restriction-001` / `WebOnt-Thing-003` shapes.
- **Parallel saturation (opt-in `par` feature, Phase E4, bead `sq-wy3i6`):** add `features = ["par"]`. `Classifier::classify_par(&dict, &triples, threads)` / `classify_graph_par(&mut dict, &mut triples, threads)` (`threads: NonZeroUsize`) run the SAME CR1–CR6 (+`rbox` CR10/CR11) rules as **deterministic bulk-synchronous rounds**: the membership frontier is partitioned across a bounded `std::thread::scope` worker pool that derives rule firings read-only against the round-start snapshot, then a sequential apply phase reuses the single-threaded `add`/`add_link` machinery. **The derived closure is IDENTICAL to the single-threaded engine at every thread count** (soundness + completeness + determinism — pinned by `tests/par_differential.rs` differentials incl. delayed-filler CR4 / CRs1-link mutation witnesses and a repeated-run determinism stress, plus the `sparq-conformance/el-suite-par` differential over the W3C EL corpus in the CI el-suite lane); emitted triples match content AND order. **OFF by default** — zero threading code without it; native targets only (do NOT enable for wasm, where `std::thread` cannot spawn — the default single-threaded path is the wasm story). No wall-clock/speedup claim is made or pinned (work-box timings are non-canonical).
- **Phase attribution (`par`, bead `sq-q0o82`):** only the **compute** phase is parallel; **apply** (`add`/`add_link`, and the `rbox` CR10/CR11 closure) is still sequential, so the E4 speedup is Amdahl-bounded by it. `classify_graph_par_stats(&mut dict, &mut triples, threads) -> (Report, ParPhaseStats)` is `classify_graph_par` plus that split — the graph mutation, emitted triples and `Report` are byte-identical (it only reads two clocks per round). `ParPhaseStats::{rounds, frontier_items, derived_members, derived_links}` are a function of the input ALONE and are **invariant across thread counts** (chunking decides *which worker* derives a conclusion, never *which* conclusions — pinned in `tests/par_differential.rs`), so they are safe to assert on; `compute_nanos`/`apply_nanos` are wall-clock, non-deterministic and **non-canonical** — use `ParPhaseStats::apply_fraction()` (the sequential share, `0.0..=1.0`) as a ratio and never quote an absolute figure. The `par_phase_bench` example (`--features par`) prints the split: with no args it runs a synthetic wide-taxonomy workload and *asserts* the closure matches sequential and the work counts are thread-invariant; with `<path> [format]` it gathers the same row for a real ontology — the input the still-OPEN **parallel-apply refinement** decision needs (a GO/SNOMED-scale dump this repo does not vendor), since a sharded apply would put the identical-closure invariant at risk and is not worth it on synthetic evidence alone.
- **Keys + negative property assertions + differentFrom (also `abox`, bead `sq-pbz04.2.8`):** the `realize` / `realize_graph` readoff ALSO reasons over three more ABox mechanisms, all sound-over-complete. **`owl:hasKey(C, keys)`** merges two DISTINCT **named (IRI)** individuals BOTH derivably in `C` that share a value on **EVERY** key property (⇒ **`a owl:sameAs b`**, in `Realization::same_as()`). Object keys match a shared **nominal successor** (`{b} ∈ R(p)[{a}]`, asserted or derived); data keys a shared **literal TERM** (identical terms ⇒ identical values — sound; value-equal-but-lexically-distinct literals are an honest incompleteness pending the `cdomain` numeric tower). Firing on a **PARTIAL** match is impossible **by construction** (each key property's shared-value set must be non-empty AND intersect — the classic HasKey over-derivation trap, pinned by a negative test; e.g. `New-Feature-Keys-006`'s single-member key never fires, and the functional-property clash it needs is honestly out of fragment). **`owl:NegativePropertyAssertion`** is a whole-ontology clash (**`is_inconsistent()`**) iff the corresponding POSITIVE is asserted or DERIVED — object NPA against a nominal successor (asserted *or* derived), data NPA against an asserted `a p "v"` triple (the check reads ASSERTED triples, NOT the `sq-vkq9u` rescued `∃q.{v}` existentials, so a *derived*-only data positive is missed: an honest incompleteness, never an unsound missed clash). **`owl:differentFrom`** (in `Realization::different_from()`, `realize_graph` materializes `owl:differentFrom` triples) is read off **ONLY** from a derived nominal clash (`{a} ⊓ {b} ⊑ ⊥` via a disjointness — `WebOnt-disjointWith-001`) or the SYMMETRIC closure of an asserted inequality (`WebOnt-differentFrom-001`) — never fabricated; a `sameAs`/`differentFrom` coincidence (`New-Feature-Keys-002`) is **inconsistent**. **Fail-closed:** a malformed key (non-atomic class, empty/non-IRI list) or NPA stays counted in `Report::skipped_assertions`. `src/abox.rs` pins the `Keys-001/-002/-003/-006`, both NPA-001, `differentFrom-001`, `disjointWith-001` shapes plus the partial-key negative test.
- **Incremental classification under TBox edits (opt-in `incremental` feature, Phase E5, bead `sq-clsv6`):** add `features = ["incremental"]`. `IncrementalClassifier::new(&dict, &triples)` classifies once and KEEPS the CR1–CR6 saturation; `apply_edits(&dict, added, removed) -> IncrementalReport` then re-classifies the edited TBox and `hierarchy() -> ClassHierarchy` / `triples()` read the post-edit state. **READ THE SCOPE BEFORE RELYING ON THE NAME.** EL saturation is a monotone least fixpoint, so only ADDITION is incremental: a **monotone extension** (new class axioms + brand-new class-expression nodes) is folded into the live fixpoint by a delta-seeded resume (`classify::resaturate` re-queues just the retained memberships a new axiom's trigger keys occur in — the least-fixpoint argument for why that equals a from-scratch closure is in its doc comment). **Any RETRACTION takes a full re-classification**, as does any RBox / concrete-domain change and any edit that attaches structure to a node the graph already MENTIONS (that can CHANGE an existing axiom rather than add one — including the first structure on a node an axiom currently reads as an opaque class atom). The saturation keeps **no derivation provenance**, so DRed / ELK affected-context deletion repair (Kazakov & Klinov, ISWC 2013) is **NOT implemented** — deferred, never faked. Which path ran is always disclosed in `IncrementalReport::disposition` (`NoOp` / `Incremental` / `Full(FullReason::{Retraction, ExistingNode, Vocabulary, ConcreteDomain})`), with `added_axioms` / `reseeded_memberships` as the work measures. **The invariant that holds on BOTH paths:** the post-edit hierarchy equals `Classifier::classify` over the post-edit triple set — pinned edit-by-edit in `tests/incremental.rs` against the hand-derived ELK-calculus closure oracle AND against a from-scratch classification, over hand-built edited-TBox fixtures plus randomized add / add-and-retract / **triple-at-a-time** streams. `Report::skipped_axioms` agrees with a from-scratch extraction too; `Report::named_classes` does NOT (it reports the live concept-index size, which also counts fresh normalization names earlier edits minted). TBox-only in every feature state (never internalizes ABox assertions), single-threaded (independent of `par`), and **OFF by default** — the stateless E1–E4 entries carry zero edit-tracking state or code. No wall-clock/speedup claim: the win is skipping the SATURATION, but every edit still rescans the graph structurally, so on a small graph the fixed costs can dominate (work-box timings are non-canonical).
- **Deferred EL fragment (honest incompleteness, surfaced — NOT silently wrong):** without `cdomain`, ALL concrete-domain shapes (`owl:onDataRange`/`owl:withRestrictions`/`owl:onDatatype`/`owl:datatypeComplementOf` + literal `hasValue`/`oneOf`) land in `Report::skipped_axioms`; with it, the unsupported remainder above still does. Distinct from constructs **outside EL entirely** (unionOf / complementOf / allValuesFrom / cardinality / a **multi-individual** `owl:oneOf` — the profile's `ObjectOneOf` admits exactly one individual, more is a disjunction — all skipped, but those need ALC / Horn-SHIQ, not a deferred EL slice; `owl:hasSelf` is NO LONGER here — it is in-fragment via CR-Self, bead `sq-pbz04.2.6`) and from RBox (a *gated* capability via `rbox`, not permanently deferred). Parallel saturation is the opt-in `par` feature (E4, bead `sq-wy3i6` — identical closure at every thread count); incremental-ADDITION re-classification under TBox edits is the opt-in `incremental` feature (E5, bead `sq-clsv6` — retraction falls back to a full re-classification, reported). `classify_graph` (full closure) and `classify_hasse_graph` (reduced) are both available — pick by whether you want every derived subsumption or just the immediate-parent taxonomy.
- **End-to-end scaling check.** `cargo run -p sparq-reason-el --features rbox,hasse --example snomed_go_scale_bench --release [SCALE]` runs a SNOMED/GO-shaped slice (is-a forest + transitive part-of + SNOMED right-identity role chain + existential restrictions) at 1×/2× and asserts a **relative** (dimensionless) property: closed-form derived counts hold at both scales (conformance) AND the work proxy doubles at most ~2× — confirming normalise + RBox + Hasse compose with **no hidden quadratic**. No hard-coded ms (work-box timings are non-canonical); `tests/snomed_go_scale.rs` is the CI-gated counterpart (runs under the `rbox`/`hasse` legs).
- **W3C OWL 2 EL suite — a pinned EXTENSION ratchet (bead `sq-pbz04.2.4`/`sq-pbz04.2.9`, opt-in `sparq-conformance/el-suite`).** The W3C OWL WG export (`tests/w3c/owl2/all.rdf`), filtered to `test:EL` ∧ `test:RDF-BASED` (Approved, inline RDF/XML premise, no `owl:imports`), is run through the **REAL** classifier: each premise is classified with `classify_graph` (materializing the complete `rdfs:subClassOf` lattice IN PLACE, with **`rbox`** + **`cdomain`** also on — the CI lane exercises the full shipped feature set) and each declared check decided — **consistency** (no unsatisfiable named class), **inconsistency** (some unsatisfiable named class), **positive-entailment** (the lattice ENTAILS the conclusion via the bnode-homomorphism `entail::entails` after output-vocabulary completions: datatype axiomatic-set + mutual-subsumption → `owl:equivalentClass` augmentation — a semantic identity that graduated WebOnt-equivalentClass-003, sq-pbz04.2.9), **negative-entailment** (non-conclusion NOT entailed). `EL_SUITE_FLOOR` is the **MEASURED PASS count** — a **`sparq extension`** row in the central scoreboard (`scoreboard::SUITES`), tallied **separately** and **NOT** a full-OWL-2-EL-conformance claim: tests needing **ABox inconsistency** (individual assertions), or a conclusion in `owl:sameAs`/`rdfs:subPropertyOf`/`owl:equivalentProperty`/`owl:TransitiveProperty`/`owl:unionOf` axiom form (output-vocabulary gaps distinct from inference gaps) are **audited PERMANENT divergences** (reported separately, **never summed into the floor**). `EL_SUITE_FLOOR` is read textually by `tests/scoreboard_floors.rs`, so the mirrored scoreboard value cannot drift; `--nocapture` prints an `OWL 2 EL ratchet pass N of M (floor F)` line the CI job `inference-conformance` re-greps.
- **CLI surface (opt-in `sparq-cli` `el` feature, Phase E6, bead `sq-2ch27`):** `cargo run -p sparq-cli --features el -- classify <data-file> <format> [out.nt]` runs the classifier and prints the report as `name<TAB>value` lines on stdout (`triples`, `named_classes`, `emitted_subclassof`, `emitted_subpropertyof`, `skipped_axioms`, `unsatisfiable_classes`, `thing_unsatisfiable`, `rbox_non_regular`), writing the lattice-augmented graph to `out.nt` when given. The same classifier is also a **reasoning profile**: `query … --reason el` and `reason <f> <fmt> el [out.nt]` classify then hand the augmented triples to the ordinary query/serialisation path. The CLI feature pulls `sparq-reason-el` **with `rbox`** (E1+E2), so role inclusions/chains/transitivity are applied and the role lattice is emitted. Everything the classifier could not reason over is printed as an explanatory NOTE on **stderr** (non-zero `skipped_axioms`, a non-regular RBox, unsatisfiable classes, `owl:Thing ⊑ owl:Nothing`) rather than swallowed. **Without the feature `--reason el` is a hard exit-2 error naming it — never a silent fall-back to `owl`**, which would return a quietly incomplete hierarchy. The other EL features (`hasse`, `cdomain`, `abox`, `par`, `incremental`) have **no CLI surface** — they are library-only today. **That bounds the CLI capability to E1+E2, short of the full OWL 2 EL profile:** with `cdomain` off, every concrete-domain axiom (faceted `owl:onDatatype`/`owl:withRestrictions`, literal `owl:hasValue`/`owl:oneOf`) is deferred into `skipped_axioms` and NOT applied, so a *valid EL* ontology using supported datatype restrictions can classify to an INCOMPLETE hierarchy under `classify` / `--reason el`. The per-run `skipped_axioms` count is honest about a given run; reach for the library API (`sparq-reason-el` with `cdomain`) when the ontology needs CR7–CR9.
- **Use EL, not `--reason owl`, when you need a class hierarchy over an EL ontology** (complete for the fragment the enabled features cover — see the CLI-surface bullet above for the `cdomain` deferral). RL is not an approximation you can tune up with more rules — EL needs a different algorithm. A verified in-repo differential (`crates/sparq-cli/tests/el_cli.rs::el_derives_what_rl_cannot`): given `B ⊑ C`, `B ⊑ E`, `C ⊓ E ⊑ D`, `--reason el` derives `B ⊑ D` (completion rule **CR2**) and `--reason owl` does not — RL's `scm-int` only DECOMPOSES an intersection, and the composition direction exists in the RL/RDF rule set only as the assertional `cls-int1`, over individuals.

## OWL 2 QL query rewriting (`sparq-reason-ql`, EXPERIMENTAL, separate opt-in crate)

OWL 2 QL (DL-Lite_R) is **FO-rewritable**: instead of materializing a closure, you **rewrite the query** into a **union of conjunctive queries** (UCQ) that, evaluated over the **unmodified data**, returns the **certain answers** under the schema (Calvanese et al., *PerfectRef*, JAR 2007). **`sparq-reason-ql`** is a query-rewriter (not a materializer): it reuses the engine's query path — it emits a rewritten `spargebra::Query` (a `Union`-folded UCQ) that the planner/executor run unchanged.

```rust,ignore
// Cargo.toml:  sparq-reason-ql = { version = "0.1", features = ["experimental"] }
use sparq_reason_ql::{rewrite, rewrite_production, as_conjunctive_query, CqError};
use spargebra::SparqlParser;

let q = SparqlParser::new().parse_query("SELECT ?x WHERE { ?x a <http://ex/Employee> }")?;
// `tbox`: &[oxrdf::Triple] carrying rdfs:subClassOf/subPropertyOf/domain/range, owl:inverseOf …
let r = rewrite(&q, &tbox)?;            // baseline PerfectRef UCQ; r.report.disjuncts / .skipped_axioms
// Production path: PerfectRef + tree-witness folding + UCQ-containment MINIMISATION (smaller UCQ,
// SAME certain answers). r.report.disjuncts_before_minimisation - r.report.disjuncts = dropped.
let p = rewrite_production(&q, &tbox)?;
// The CQ-shape gate alone (no feature needed) classifies a query without rewriting:
match as_conjunctive_query(&q) { Ok(_cq) => {}, Err(CqError::OutOfScope(why)) => { let _ = why; } }
```

- **FAIL-CLOSED CQ-shape gate (the soundness keystone) with broadened sound fragment.** PerfectRef is sound + complete only for **conjunctive queries**. The gate is **fail-closed** and now accepts a BROADENED sound fragment: **(B1)** top-level UCQ (`UNION` of CQ branches, each rewriting independently; `as_ucq`); **(B2)** RDF literal constants in role-atom object position (rigid, never `_`-eligible — the applicability condition is unchanged); **(B3)** `FILTER` over **distinguished-only** variables (passed through, fail-closed for non-distinguished vars); **(B4)** constant-only `VALUES` over distinguished variables (re-applied as an inner join, fail-closed for UNDEF / non-distinguished); **(B3/B4 per-branch, sq-sg542)** in a MULTI-branch UCQ — a hand-written `{ … FILTER } UNION { … }`, or an alternation path whose `#1671` desugaring distributes a top-level `FILTER` into EACH branch — the emitter is **branch-aware** (`emit::ucq_to_pattern_per_branch`): each branch emits its OWN `FILTER`/`VALUES` over its OWN sub-union, so a branch's modifier constrains only that branch (never hoisted over the whole union, never silently dropped — the leak is differential-tested in BOTH directions in `tests/branch_aware_emit.rs`); this shape was previously rejected fail-closed; **(Bbnode, sq-pbz04.3.6)** body blank nodes in subject/object positions are lifted to fresh existential variables — each distinct blank-node LABEL in a CQ maps to one unique `Unbound` id (shared labels get the same id, so `is_bound_var` correctly treats the shared position as bound and blocks the existential applicability condition on it; distinct labels get distinct ids). The **emitter** completes the round-trip: a repeated `Unbound` id maps to ONE output variable, so a shared body blank node emits as a genuine **JOIN** (not a cartesian product over-approximation) — the load-bearing invariant that lets the shared-existential SELECT cases (`sparqldl-07`/`-08`) graduate rather than fall to `oracle-divergent`. **(B5, sq-pbz04.3.2)** non-recursive property paths — sequence `p1/p2` (fresh non-distinguished intermediate, shared → a JOIN), inverse `^p` (subject/object swap), alternation `p1|p2` (branch multiplication into the B1 UCQ machinery) — are desugared to an equivalent CQ/UCQ BEFORE the entailment rewrite, after which PerfectRef soundness applies (STEP-0-verified: the vendored spargebra parser already lowers a top-level sequence/inverse to a BGP, so only a surviving `Path` alternation is rewritten — no double-translation); `+`/`*`/`?` (recursion / zero-length) and negated property sets stay fail-closed. **(B6)** `rdfs:subClassOf`, `rdfs:subPropertyOf`, `rdfs:domain`, `rdfs:range`, and all `owl:` predicates used as **atom predicates** are **rejected** (intensional-atom guard — fail-closed); annotation predicates (`rdfs:label/comment/seeAlso/isDefinedBy`) are admitted. Everything outside this fragment — `OPTIONAL`/`MINUS`/**recursive-or-negated** property paths (`+`/`*`/`?`/`!p`)/aggregation/variable-predicate/non-distinguished FILTER or VALUES — is **rejected as `CqError::OutOfScope(reason)`**, never silently mis-answered. The applicability condition (an existential generator fires only on an UNBOUND, non-distinguished, non-shared variable) is enforced explicitly. The `reduce` MGU treats **distinguished (answer) variables as rigid** — it never identifies two answer columns.
- **Scope (`experimental`):** the **positive** DL-Lite_R inclusions — `rdfs:subClassOf`/`subPropertyOf`, `rdfs:domain`/`range` (`∃R ⊑ A`, `∃R⁻ ⊑ A`), `owl:inverseOf`, `owl:equivalentClass`/`equivalentProperty` (decomposed to inclusion pairs, named operands only), and **unqualified** `∃R` `owl:someValuesFrom owl:Thing` restrictions. Non-QL axioms are counted in `RewriteReport::skipped_axioms`, never applied.
- **TBox-capture accounting (`TBox` struct, `sq-pbz04.3.3`).** `TBox::extract(triples)` now tallies every `rdfs:`/`owl:` triple it sees: positive inclusions go to `concept_incl`/`exists_super`/`role_incl`; `skipped` counts non-QL constructs (unchanged); **`consistency_relevant`** counts QL-legal negative/disjointness axioms (`owl:disjointWith`/`propertyDisjointWith`/`complementOf`) — present but never applied for REWRITING; `owl:disjointWith`/`propertyDisjointWith` with resolvable operands are additionally captured STRUCTURALLY into **`neg_incl`** (input to the opt-in `ql-consistency` check, sq-p6yb7) with the residue counted in **`consistency_uncaptured`** (invariant: `consistency_relevant == neg_incl.len() + consistency_uncaptured`); so is the **subClassOf-complement** shape `A rdfs:subClassOf [ owl:complementOf B ]` (sq-fj8lj follow-up) — QL's `superClassExpression ::= ObjectComplementOf(subClassExpression)`, i.e. exactly the negative inclusion `A ⊑ ¬B`, so it is counted at the `rdfs:subClassOf` axiom (consistency-relevant, NOT `skipped`; the complement blank node's own `owl:complementOf` triple is then not tallied again) with `∃R` operands resolving on either side, while a **NAMED-subject** `A owl:complementOf B` still asserts the stronger biconditional `A ≡ ¬B` and stays uncaptured; **`unrecognised_schema`** counts OWL/RDFS constructs the extractor does not classify — either an `rdfs:`/`owl:`-predicate triple not handled above, or an `rdf:type` triple whose object is unmodelled schema vocabulary (e.g. `:p rdf:type owl:FunctionalProperty`); `fully_captured()` can therefore be `false` even when all *predicate* IRIs are outside the `rdfs:`/`owl:` namespace. **`TBox::fully_captured()`** returns `true` iff `skipped == 0 && unrecognised_schema == 0` — an accounting/honesty signal that no schema triple was silently dropped, not a DL-Lite_R completeness proof. A `consistency_relevant > 0` count does not block `fully_captured()`.
- **Production path (`rewrite_production`):** baseline PerfectRef **augmented** with bounded **tree-witness** folding (existential witnesses captured with no unbounded chase) then **UCQ-containment minimisation** (drop disjuncts contained in a retained one). Same certain answers as `rewrite`, in a smaller UCQ. **Minimisation is FAIL-CLOSED:** containment is NP-complete, the homomorphism search is bounded, and an **undecided-within-budget** check **KEEPS** the disjunct — minimisation only ever removes a disjunct **proven contained**, so it removes no answers.
- **Oracle-tested; the FORMAL DL-Lite_R suite GRADUATED to a pinned floor (sq-qo1a9).** Validated against a hand-checked DL-Lite_R oracle (`sparq-reason-ql/tests/oracle.rs`, incl. tree-witness + minimisation cases), because no Rust PerfectRef reference exists to diff against. **DL-Lite_R consistency checking is opt-in** (`ql-consistency` feature, sq-p6yb7): `check_consistency`/`check_consistency_with` compose a boolean violation query per captured negative inclusion, rewrite it through the SAME PerfectRef saturation, and evaluate it over the data — **INCONSISTENT** iff some violation query matches (sound at any capture level, by monotonicity); definitive **CONSISTENT** only when the TBox is `fully_captured()` AND `consistency_uncaptured == 0` (the Calvanese-et-al. cln(T) completeness argument, written out in `src/consistency.rs`); fail-closed **Unknown** otherwise — an inconsistent KB certain-answers EVERYTHING, so consumers must treat the verdict, not the positive UCQ answers, as definitive. Oracle-tested in `tests/consistency_oracle.rs` (hand-derived verdicts incl. anonymous-canonical-witness violations). On the **formal DL-Lite_R suite** — the hand-derived certain-answer oracle from `sq-g19x0`, every case a conjunctive query within sound rewriting — the rewrite is **sound AND complete case by case**: `rewrite_production`'s UCQ, evaluated over the **unmodified ABox** through the real engine, returns **exactly** the hand-derived certain answers. That is now a **pinned floor** (`sparq-conformance`'s `tests/ql_dllite_suite.rs`, opt-in `ql-experimental`; `QL_DLLITE_FLOOR = 11` sound-and-complete cases), registered as a **`sparq extension`** row in the central scoreboard (`scoreboard::SUITES`) and tallied **separately** — **NOT folded into the standards-conformance total**, and **NOT a full-OWL-2-QL-conformance claim** (there is no runnable normative W3C QL certain-answer suite; the W3C QL material is structural). Like the RIF-Core / RSP / BM25 extension rows, it pins a faithful sparq-OWN oracle. `QL_DLLITE_FLOOR` is read textually by `tests/scoreboard_floors.rs`, so the mirrored scoreboard value cannot drift.
- **The `pr:QL` `sparql11/entailment` arm: the SOUND subset is GRADUATED to a pinned named-case floor (sq-pbz04.3.4); the rest stays held with an exhaustive reason taxonomy (sq-kuvu3; opt-in `sparq-conformance/ql-experimental`).** Every `sd:EntailmentProfile pr:QL` case runs through a **six-condition graduation predicate** (`inference::sparql_entail::run_ql_graduation`), each condition **checked in code, never assumed**: (1) the fail-closed CQ-shape gate accepts the query AND it carries no intensional schema-vocabulary atom (B6, sq-pbz04.3.1 — now built into the gate); (2) the TBox is **totally captured** (`fully_captured()`, sq-pbz04.3.3); (3) the **consistency condition** — zero consistency-relevant (negative/disjointness) axioms, OR (sq-p6yb7) the DL-Lite_R violation-query consistency check proves the KB **CONSISTENT** (a proven-INCONSISTENT KB holds at `inconsistent-kb` — entailment-regime behaviour on an inconsistent graph is implementation-defined, never a guessed everything-entailed pass — and an UNKNOWN verdict holds at `pending-consistency`); (4) default-graph dataset only; (5) the **regime-coincidence guard** — the crate computes CERTAIN ANSWERS while W3C entailment-regime solution mappings bind every variable to an RDF term; the semantics provably coincide iff all body terms are distinguished (a body blank node counts as a non-distinguished variable) OR the TBox has no existential-generating inclusion (`exists_super` empty) — the fail-closed §4 default, deliberately not widened; (6) the rewritten UCQ evaluated over the **unmodified data** is **result-equivalent to the W3C oracle**. The graduated cases form the **pinned named-case floor** `QL_ENTAILMENT_FLOOR_CASES` in `sparq-conformance`'s `tests/ql_entailment_floor.rs` (exact set equality: a regressing pinned case AND an unpinned newly-eligible case both fail — additions need an evidence-carrying PR; enforced in the `inference-conformance` CI job), mirrored as a **`sparq extension`** scoreboard row (`QL_ENTAILMENT_FLOOR`, read textually by `tests/scoreboard_floors.rs`) — **NEVER summed into the standards-conformance total, NOT a full-regime/full-profile OWL 2 QL conformance claim**. Every non-graduated case carries a specific taxonomy hold: **permanently-outside** (BIND / variable predicates / intensional schema queries / OPTIONAL–MINUS shapes — no sound rewriting in this design), **pending-gate** (B1/B2/B3/B4/B6 landed under sq-pbz04.3.1; **(B5)** non-recursive property-path desugaring — sequence/inverse/alternation — now landed under sq-pbz04.3.2, so a path-shaped CQ is no longer held at the gate; recursive/zero-length/negated paths stay fail-closed; **(Bbnode)** body blank nodes lifted to fresh existential variables so the applicability condition applies correctly — sq-pbz04.3.6), **pending-capture**, **pending-consistency** (now ONLY structurally-uncaptured negative axioms, e.g. a NAMED-subject `owl:complementOf` — the subClassOf-complement spelling `A rdfs:subClassOf [ owl:complementOf B ]` IS captured, sq-fj8lj follow-up; the bucket measured 0 BEFORE the sq-p6yb7 check landed, so the upgrade graduates no case at the current rdf-tests pin), **inconsistent-kb**, **pending-coincidence**, **oracle-divergent**, or **inconclusive** — plus a loud **unclassified-abstain** bucket the floor test asserts EMPTY, so no new abstain class can hide in a catch-all. In the inference BINARY every QL row (graduated or held) remains OutOfScope — no QL row can inflate the binary's conformance ratchet (the D-entailment precedent); `tests/ql_experimental_arm.rs` asserts exactly that plus the taxonomy invariants.

## OWL 2 Direct Semantics (`sparq-reason-dl`, separate opt-in crate — all five layers built: L1 model, L2 profile checker, L3 ALCH tableau, L4 dispatch, L5 conformance arm)

The three profile reasoners above (RL / EL / QL) each cover a *tractable* OWL fragment; **OWL 2 Direct Semantics** (the model-theoretic DL semantics) covers the boolean heart of DL — arbitrary `⊔` / `¬` / `∀` — that none of them can reach. `sparq-reason-dl` is a **separate opt-in crate** building a **layered, fail-closed Direct-Semantics checker**; **all five layers are built: L1 (structural model + extractor), L2 (syntactic EL/QL/RL profile checker), L3 (ALCH tableau — the first layer that does semantic reasoning), L4 (the fragment-dispatch `DirectChecker` + entailment-by-refutation, behind the crate's opt-in `dispatch` feature, bead sq-pbz04.4.4), and L5 (the `sparq-conformance` DIRECT-arm behind that crate's opt-in `dl-direct` feature, bead sq-pbz04.4.5)**. HONEST SCOPE: this is **not** full OWL 2 DL (SROIQ(D) satisfiability is 2NEXPTIME-complete and deliberately out of scope) — it is a scoped **ALCH-fragment** effort, sound/complete only within the argued fragment. L1 delivers:

- **A structural OWL model** (`sparq_reason_dl::model`) — `Axiom` / `ClassExpression` / `ObjectPropertyExpression` typed enums for the ALCH fragment: named classes, `owl:Thing`/`owl:Nothing`, `owl:intersectionOf` (⊓), `owl:unionOf` (⊔), `owl:complementOf` (¬), `owl:someValuesFrom` (∃R.C) and `owl:allValuesFrom` (∀R.C) over **named object properties**; GCIs, `owl:equivalentClass`, `owl:disjointWith`, `rdfs:subPropertyOf`, `rdfs:domain`/`rdfs:range`, and a ground ABox. Purely structural — **no semantics attached at L1**.
- **A FAIL-CLOSED reverse RDF mapping** — `extract(&Dict, &[[Id; 3]]) -> Result<Ontology, ExtractError>` maps the `(Dict, triples)` substrate into the model per the W3C *Mapping to RDF Graphs* tables restricted to ALCH. **A single out-of-fragment or malformed triple aborts the WHOLE extraction** with a typed `ExtractError`, rather than being silently dropped: the (future) checker must never reason over a graph it only *partially* understood — a dropped axiom can flip a consistency verdict. Understood in full, or refused. The rejection taxonomy has five arms — `OutOfFragment` (cardinality / nominals / inverses / `owl:sameAs` / property characteristics / chains / keys), `DataConstruct` (datatypes / data properties — no concrete domain in L1), `MalformedList`, `MalformedClassExpression`, `Unclassifiable` (an undeclared predicate that cannot be mapped soundly) — while annotations, declarations, and ontology headers are recognised and ignored.
- **Forward RDF renderer** (`render`, bead sq-pbz04.4.7) — `render_to_triples(&Ontology, &mut Dict) -> Vec<[Id; 3]>` maps the structural model BACK to OWL RDF triples (the inverse of `extract`), enabling full-fragment round-trip testing (`RDF → extract → render → extract` yields the same structural model; blank-node identity may differ but `Ontology: PartialEq` holds). `render_to_turtle(&Ontology, &mut Dict) -> String` serialises the same output to minimal Turtle for human-readable diagnostics. No extra dependencies; always compiled (not feature-gated). The round-trip invariant is checked at TWO tiers: the hand-written `render::tests` fragments (incl. two regression cases for the sq-pbz04.4.17 fix below), and — belt-and-suspenders — EVERY ontology document of the W3C DIRECT-arm corpus via `inference::dl_suite::run_render_roundtrip_arm` in `sparq-conformance` (opt-in `dl-direct` feature, bead sq-pbz04.4.17; `RenderRoundTripReport` with EXACT-pinned counts — `DL_RENDER_ROUNDTRIP_FLOOR` — and a hard EMPTY-violations assertion: a mis-render is a REAL fidelity bug, never a pinnable divergence). The corpus arm's first run CAUGHT and fixed one: a named-composite `EquivalentClasses(A, expr)` used to render as an INLINE backbone on `A`, which re-extracts differently whenever `A` is referenced elsewhere (14 corpus mismatches) and refuses outright on a self-referential definition (`WebOnt-someValuesFrom-003`); the renderer now always emits the explicit `A owl:equivalentClass _:b` shape, which round-trips both origins. sq-pbz04.4.18 completes the corpus sweep with `run_render_roundtrip_rdf_based_arm`, the same invariant over the DISJOINT **RDF-BASED-only** slice (`test:RDF-BASED` without `test:DIRECT`), pinned separately via `DL_RDF_BASED_ROUNDTRIP_FLOOR`. Admissible because the round trip is purely syntactic — it makes no semantics claim, so no reasoning verdict is ever attributed to an RDF-BASED test. MEASURED, correcting the bead's "several hundred more documents" estimate: **479 of the export's 493 cases are DUAL-tagged** and were already covered, so the slice is just **7 cases / 13 documents** (+1.9%); 10 round-trip, 3 are refused at L1, and violations are 0 — it found no new fidelity bug. Coverage is exhaustive over the ELIGIBLE slice — every non-`Rejected` case that carries a recognised check kind (consistency / inconsistency / positive- or negative-entailment / profile-identification) and is sanctioned `test:DIRECT` and/or `test:RDF-BASED` — NOT over the raw export: the 1 case tagged with NEITHER semantics, and any case carrying no recognised check kind, is selected by neither arm and its documents stay unmeasured.

**L2 — syntactic EL/QL/RL profile-membership checker (`profile`, bead sq-pbz04.4.2):** NOW
BUILT. `profile::profiles(onto: &Ontology) -> ProfileSet` runs a purely syntactic grammar walk
(W3C OWL 2 Profiles §2/§3/§4) over the structural model and returns a `ProfileSet` with three
fields — `el`, `ql`, `rl` — each a `Membership` enum: `Membership::In` (all axioms pass),
`Membership::NotIn(reason)` (first violation, fail-fast), or `Membership::Unknown(err)`
(extraction failure, only from `profile::profiles_from_extraction(&Result<Ontology, ExtractError>)`).
Convenience methods: `Membership::is_in()` / `is_not_in()` / `is_unknown()`;
`ProfileSet::in_all()` / `in_any()`. An empty ontology is `In` all three profiles. Terminating
by construction — no semantic reasoning, just a grammar walk over the finite acyclic structural model.

**L3 — terminating ALCH tableau (`nnf` + `tableau`, bead sq-pbz04.4.3):** NOW BUILT — the
consistency / class-satisfiability core. `tableau::consistency(&Ontology, Budget)` and
`tableau::class_satisfiability(&ClassExpression, &Ontology, Budget)` return a tri-state
`Verdict` — `Satisfiable`, `Unsatisfiable`, or `Unknown(UnknownReason)` — and
`tableau::consistency_from_extraction(&Result<Ontology, ExtractError>, Budget)` is the
fail-closed RDF-level entry: ANY extraction failure yields `Unknown(OutOfFragment)` BEFORE the
tableau starts (the checker never reasons over a partially-understood graph). The engine is a
completion-forest tableau with GCI internalisation, `⊓`/`⊔`/`∃`/`∀`/GCI rules matched modulo
the `rdfs:subPropertyOf` closure, **ancestor subset blocking** (sufficient precisely because
ALCH has no inverse roles), and chronological backtracking over `⊔`-branches. The full
termination / soundness / completeness argument — citing Baader–Sattler 2001, including why
subset blocking would be insufficient with inverses — is reproduced in the `tableau` module
docs (§3–§5). Budgets are **deterministic counts only** (`Budget { max_nodes,
max_rule_applications }`; wall-clock budgets banned); exhaustion yields
`Unknown(ResourceBudget)`, **never a verdict**. `nnf` provides the negation-normal-form
rewrite (`nnf` / `nnf_complement` / `is_nnf`) and the finite `subexpression_closure` the
termination argument rests on. HONEST BOUNDARY: verdicts are sound/complete ONLY for the exact
ALCH fragment (named classes, ⊤/⊥, ⊓/⊔/¬, ∃/∀ over named properties, GCIs, `subPropertyOf`,
ground ABox) — never beyond it; the implementation is not claimed worst-case optimal (ALC+GCI
satisfiability is EXPTIME-complete).

**Opt-in transitive roles (`dl_transitive` cargo feature, OFF by default, bead sq-zfwzq
[GPT-5.6]):** extends the fragment to **ALCH + transitive roles** (Horrocks–Sattler *S with
role hierarchies* — still NO inverses / cardinality / nominals, which stay fail-closed): L1
recognises `owl:TransitiveProperty` as the feature-gated `Axiom::TransitiveObjectProperty`
(instead of refusing it), L2 classifies it per the profile grammars (IN EL §2, NOT-in QL §3,
IN RL §4), and the L3 tableau adds the **∀₊-propagation rule** (`∀R.C` at `x`, edge `x –S→ y`,
transitive `T` with `S ⊑* T ⊑* R` ⇒ add `∀T.C` at `y`) with the termination / soundness /
completeness argument EXTENDED AND WRITTEN OUT in `tableau.rs` module docs **§5a** (subset
blocking is UNCHANGED — sufficient precisely because there are still no inverses; the model
construction interprets `R^I = E(R) ∪ ⋃ E(T)⁺`). L4 dispatch routes any transitive ontology
STRAIGHT to the tableau (the only transitivity-complete branch; the RL/EL guards also
recognise the axiom kind fail-closed as defence in depth); a transitivity CONCLUSION in
entailment is decided by the two-step-chain refutation encoding (`O ⊨ Trans(R)` iff
`O ∪ {R(a,b), R(b,c), B(c), (∀R.¬B)(a)}` unsatisfiable — argued in `check.rs`). With the feature
enabled, a declaration-free conclusion role assertion may reuse a role kind established by
a transitivity-bearing premise; the checker adds only semantically inert declarations for
premise-confirmed roles during conclusion extraction and never guesses an unknown predicate.
With the feature OFF the crate compiles to exactly the pre-extension code (fail-closed refusal). The
`sparq-conformance` `dl-direct` arm enables it, graduating the corpus's transitive
consistency/entailment cases from abstentions to definitive verdicts (floors re-pinned with
evidence in `tests/dl_suite.rs`).

**L4 — fragment-dispatch checker + entailment-by-refutation (`check`, opt-in `dispatch`
feature, bead sq-pbz04.4.4):** NOW BUILT. `check::DirectChecker` (constructed with `new()` or
`with_budget(Budget)`) dispatches an extracted ontology IN ORDER — RL (via `sparq-reason`
materialization + clash scan, Theorem-PR1-precondition-CHECKED, divergence-guarded), EL (via
`sparq-reason-el`, triple-guarded: skipped-axioms / unapplied-axiom-kinds / ⊤-guard), QL
(consistency wholly deferred to sq-pbz04.3.4 — always abstains), else the L3 ALCH tableau —
returning `ConsistencyOutcome` / `EntailmentOutcome`: a tri-state verdict PLUS the `Branch`
that produced it (traceability). `entailment()` checks `O ⊨ α` per conclusion axiom by an
argued refutation encoding onto the tableau (`SubClassOf`, `ClassAssertion`,
`ObjectPropertyAssertion` via a fresh-class encoding, `EquivalentClasses`, `DisjointClasses`,
domain/range, and — since sq-pbz04.4.9 — `SubObjectPropertyOf(R,S)` via the
fresh-individual-pair lift `{R(a,b), B(b), (∀S.¬B)(a)}`, sound and complete because the
tableau's ∀-rule fires modulo the role hierarchy). Every guard fails CLOSED — uncertainty is a
typed `UnknownReason`, never a guessed verdict. **Refutation budget fallback (sq-pbz04.4.10):**
the tableau still OWNS every refutation (it is the only branch complete for the whole fragment),
but when — and only when — one exhausts the deterministic count budget, the SAME question is
re-asked of the RL/EL branches under their SAME guards, with the augmented model serialised by
the L1 forward renderer and its round-trip VERIFIED per call (re-extract, compare axiom
multisets). Strictly abstention-reducing: it can only replace `Unknown(ResourceBudget)` with a
definitive verdict, and any fallback abstention keeps the tableau's original budget reason. In
practice it is the RL branch that recovers — every encoding adds an ABox assertion the EL
classifier does not apply, so the EL arm always abstains today. **Conclusion anonymous individuals
(sq-pbz04.4.13):** a blank-node individual in the CONCLUSION is read EXISTENTIALLY (per the
official Direct-Semantics tests) — L1's skolem-constant reading is entailment-preserving on the
premise but would certify a WRONG `NotEntailed` on the conclusion, so before the refutation loop a
TREE-shaped anonymous assertion set (`a p _:x`, `_:x` typed / chaining to more blank nodes) **rolls
up** into an existential class assertion `a : ∃p.(⊓ types ⊓ ⊓ ∃q.⟨child⟩)` the tableau decides
SOUNDLY in both directions (so `somevaluesfrom2bnode` / `WebOnt-someValuesFrom-003` graduate to
genuine passes); any non-rollable shape — shared between two assertions, cyclic, a named/nominal
successor, or an unanchored free-existential root — abstains fail-closed
(`ConclusionAnonymousIndividual`), never a skolem `NotEntailed`.

**L5 — the conformance DIRECT-arm (`sparq-conformance`, opt-in `dl-direct` feature, bead
sq-pbz04.4.5):** NOW BUILT. `inference::dl_suite::run_direct_arm` runs the DIRECT-sanctioned
arm of the OWL WG export (`tests/w3c/owl2/all.rdf`) with **tri-state accounting
`{Pass, Fail, OutOfFragment(reason)}` — an abstention is NEVER a pass**: a
profile-identification lane (the L2 checker vs the export's POSITIVE `test:profile` tags
ONLY — the explicit-negative `owl:NegativePropertyAssertion` direction was MEASURED and not
adopted, because L2's `In` is fragment-grammar membership and cannot refute full-profile
membership (runner module docs); nothing inferred from a missing tag; the `test:species`
DL/FULL check stays deferred) and a Direct
consistency/inconsistency/entailment lane (the L4 `DirectChecker` under a PINNED
deterministic count budget). Floors are EXACT-pinned in `tests/dl_suite.rs`
(`DL_PROFILE_FLOOR` / `DL_DIRECT_FLOOR`, `==` not `>=`, so abstention-inflation and
regression both fail), mirrored as **`sparq extension`** scoreboard rows labelled **scoped
fragment — NOT full OWL 2 DL**, never folded into standards-conformance totals; the
dual-tagged tests' RDF-Based runs stay in the RL `owl_suite` / `el-suite` lanes (separate
semantics). Functional-syntax-only inputs (27 cases) and `owl:imports` are OutOfScope;
`test:status test:Rejected` is excluded.

**Scoped-fragment decision table — NOT full OWL 2 DL (grounded in code).** Deferral ledger
(live source of truth): `sparq-conformance/tests/dl_suite.rs` — `DOCUMENTED_DIVERGENCES`
(5 named rows; audited mechanisms M3/M5/M6; M1/M2/M4 FIXED and removed from the pin),
abstention counters `DL_DIRECT_ABSTAINED` / `DL_PROFILE_ABSTAINED`, pass floors
`DL_DIRECT_FLOOR` / `DL_PROFILE_FLOOR` — all EXACT-pinned (`==` not `>=`; both inflation and
regression fail CI). Design record: `research/owl2-direct-semantics-scoping.md`. [OPUS-4.8]
sq-pbz04.4.6

**L1 extraction boundary** (`extract.rs` `ExtractError` — one out-of-fragment triple refuses
the whole graph, never a partial extraction):

| Construct | L1 outcome | `ExtractError` variant |
|---|---|---|
| Named classes; ⊤/⊥; ⊓/⊔/¬; ∃R.C/∀R.C over a named property; GCIs; SubProperty; domain/range; ground ABox (`ClassAssertion`, `ObjectPropertyAssertion`) | Accepted | — |
| Cardinality (min/max/exact/qualified) | Refused | `OutOfFragment` |
| Nominals (`owl:oneOf`, `owl:hasValue`, `owl:hasSelf`); inverse properties (`owl:inverseOf`); property characteristics (Transitive/Functional/IFP/Sym/Asym/Refl/Irr) | Refused | `OutOfFragment` |
| `owl:sameAs` / `owl:differentFrom`; property chains; keys; `owl:disjointUnionOf` | Refused | `OutOfFragment` |
| Datatypes / data properties / data-range restrictions; a bare datatype-map IRI (`xsd:*`, `rdfs:Literal`, `owl:real`/`rational`) in ANY class position incl. an `rdfs:range`/`rdfs:domain` object (sq-pbz04.4.9) | Refused | `DataConstruct` |
| Malformed RDF list (unterminated, cyclic, branching, empty, orphan cell, `rdf:nil` as list cell) | Refused | `MalformedList` |
| Malformed class expression (missing filler/property, conflicting shapes, cyclic, bare blank) | Refused | `MalformedClassExpression` |
| Undeclared predicate (role-vs-annotation ambiguous); RDF 1.2 triple term | Refused | `Unclassifiable` |

**L4 dispatch — consistency** (`check.rs` `UnknownReason`; in-order, non-falling-through;
`Branch` set for traceability on every verdict):

| Branch | Decides `Consistent` | Decides `Inconsistent` | Abstains (`UnknownReason`) |
|---|---|---|---|
| RL (in-RL; PR1 preconditions pass; no divergence-guarded construct) | Yes (past divergence guard) | Yes (PR1-checked) | `RlPr1Preconditions`, `RlDivergenceGuard` |
| EL (in-EL; ⊤-free TBox; no ABox; no skipped/unapplied axioms) | Yes (empty-interpretation model construction) | Never | `ElSkippedAxioms`, `ElUnappliedAxioms`, `ElTopGuard` |
| QL (in-QL; opt-in `dispatch_ql`, sq-fj8lj → sparq-reason-ql's `ql-consistency` checker over the raw triples) | Only past the QL crate's OWN capture accounting (`fully_captured()` ∧ `consistency_uncaptured == 0`; L2's `In` only routes, never justifies) | Yes (violation query matched — sound at any capture level by monotonicity) | `QlCaptureGap` (the QL crate's gap accounting); without `dispatch_ql`: `QlConsistencyPending` (always) |
| ALCH (all else the L1 extractor accepted) | Yes (complete for L1 fragment) | Yes (complete for L1 fragment) | `ResourceBudget` |

**L4 dispatch — entailment** (all conclusion kinds routed through the complete ALCH tableau):

| Conclusion kind | Decides `Entailed` / `NotEntailed` | Abstains (`UnknownReason`) |
|---|---|---|
| `SubClassOf`, `ClassAssertion`, `EquivalentClasses`, `DisjointClasses`, `ObjectPropertyDomain` / `ObjectPropertyRange` | Yes (sound + complete via refutation encoding) | — |
| `ObjectPropertyAssertion` (fresh-class encoding — sound and complete, `check.rs` §4) | Yes | — |
| `SubObjectPropertyOf` (fresh-individual-pair encoding `{R(a,b), B(b), (∀S.¬B)(a)}` — sound and complete; sq-pbz04.4.9) | Yes | — |
| Tree-shaped conclusion blank node (rolls up to an existential class assertion; sq-pbz04.4.13) | Yes | — |
| Non-tree conclusion blank node (shared / cyclic / named-successor / free-existential root) | Never | `ConclusionAnonymousIndividual` |
| A future axiom kind without an argued encoding (none expressible today) | Never | `UnencodedConclusion` |
| Deterministic count budget exhausted mid-search | Never | `ResourceBudget` |

Deferred constructs — inverse roles, cardinality/functionality, nominals, transitivity,
`sameAs`/`differentFrom`, datatypes, keys — are each **rejected, never mis-mapped**, with a
named reason and unlock path in the deferral ledger: `sparq-conformance/tests/dl_suite.rs`
(`DOCUMENTED_DIVERGENCES`, `DL_DIRECT_ABSTAINED`, `DL_PROFILE_ABSTAINED`) and the design
record `research/owl2-direct-semantics-scoping.md`.

## Common recipes

**1. OWL 2 RL closure + inconsistency report.** OWL includes RDFS; run the clash check on the materialized result.

```rust
use sparq_core::Graph;
use sparq_reason::{materialize, inconsistencies, Profile};

let (mut dict, mut triples) = Graph::parse_to_triples(owl_text, "turtle")?;
materialize(Profile::OwlRl, &mut dict, &mut triples);
let clashes = inconsistencies(&dict, &triples);   // e.g. disjointWith / sameAs↔differentFrom
if !clashes.is_empty() { eprintln!("INCONSISTENT: {clashes:?}"); }
let g = Graph::from_parts(dict, triples);
# Ok::<(), String>(())
```

**2. Notation3 rules + facts in one document → entailed ground triples.** The rules and data live in the same N3 source; only ground facts survive the closure. RDF 1.2 quoted-triple TERMS (`<< s p o >>` / `<<( s p o )>>`) are first-class in rule bodies AND heads: premises match them structurally (variables inside the quotation bind, nesting included), heads derive them, and `reason_n3` interns ground triple terms via the Dict's content-addressed RDF 1.2 triple-term path (GH #2012). The opt-in `compiled-rules` path also matches them at the id level (`sq-6d43t`, see the compiled-rules section for the exact envelope); the incremental counting profile still disqualifies on them and falls back to the text engine.

```rust
use sparq_core::dict::Dict;
use sparq_reason::reason_n3;

let n3 = r#"
@prefix : <http://ex/> .
{ ?x a :Human } => { ?x a :Mortal } .
:socrates a :Human .
"#;
let mut dict = Dict::new();
let closure = reason_n3(&mut dict, n3)?;          // includes  :socrates a :Mortal
for t in &closure { println!("{} {} {} .", dict.term(t[0]), dict.term(t[1]), dict.term(t[2])); }
# Ok::<(), String>(())
```

**3. Incremental RDFS maintenance** (data updates cost time ∝ the change, not a full re-materialize):

```rust
use sparq_reason::MaterializedGraph;
let mut g = MaterializedGraph::new(&mut dict, &base_triples);
g.insert(&[[alice, ty, person]]);   // closure updated incrementally (one sweep over the delta)
g.delete(&[[alice, ty, person]]);   // exact-count retraction; a still-derivable fact survives
assert!(g.contains(&[alice, ty, agent]));   // entailed via subClassOf
let materialized: Vec<[u64; 3]> = g.closure();   // feed back into Graph::from_parts to query
```

> Note: an `insert`/`delete` touching a **TBox** triple (`subClassOf`/`subPropertyOf`/`domain`/`range`, and the OWL schema predicates) triggers a full rematerialize — watch `g.full_rebuilds()`. Keep ABox edits and schema edits separate when you care about incremental cost.

**4. Incremental N3 with a fallback check.** Term-level facts; verify you actually got the fast path.

```rust
use sparq_reason::{MaterializedN3Graph, n3::Term};
let rules = "{ ?x <http://ex/p> ?y } => { ?y <http://ex/q> ?x } .";
let mut g = MaterializedN3Graph::new(rules, &base_facts)?;   // base_facts: &[[Term;3]]
g.insert(&[[Term::Iri("http://ex/a".into()),
            Term::Iri("http://ex/p".into()),
            Term::Iri("http://ex/b".into())]]);
if let Some(why) = g.fallback_reason() { eprintln!("running batch fallback: {why}"); }
let closure: Vec<[Term;3]> = g.closure();
# Ok::<(), String>(())
```

**5. Derivation proof (`explain` feature).** Returns ONE witness derivation of a triple from the asserted base; render as text or JSON.

```rust
// Cargo.toml:  sparq-reason = { version = "0.1", features = ["explain"] }
use sparq_reason::MaterializedGraph;
let g = MaterializedGraph::new(&mut dict, &base);
if let Some(tree) = g.why(&dict, [alice, ty, agent]) {
    println!("{}", tree.to_text());   // indented, root first; rule ids like cax-sco / rdfs9 / prp-trp
    let json = tree.to_json();        // {"root":R,"nodes":[{"id":..,"conclusion":[s,p,o],"rule":..,"premises":[..]}]}
}
```

**6. `log:semantics` / `log:content` document access.** The engine does NO I/O of its own; supply a `Resolver` closure to decide what an IRI may dereference to (otherwise those builtins simply don't fire):

```rust
use sparq_reason::reason_n3_terms_with_resolver;
let resolver = |iri: &str| std::fs::read_to_string(iri.trim_start_matches("file://")).ok();
let closure = reason_n3_terms_with_resolver(src, Some("http://ex/"), Some(&resolver))?;
# Ok::<(), String>(())
```

**Import cycles always terminate (with a LIVE resolver).** N3 is Turing-complete, so a `log:semantics` document whose closure re-imports a document active up the resolution stack — directly (`A→A`), indirectly (`A→B→A`), or via a re-used node in a diamond — would otherwise spin forever once a real (filesystem/network) resolver is wired (the offline conformance harness never hit it). The engine tracks the formulae whose closure is in progress; re-entering one already in progress returns it **unclosed** (cwm's "a document already being loaded is not re-loaded") instead of recursing, so reasoning terminates. A diamond that re-uses a shared document across **sibling** branches is not a cycle and still resolves on every branch — only the pathological cyclic case changes; valid acyclic imports are byte-identical to before.

**Same-box materialization comparison (sparq vs Jena / VLog / Nemo).** To compare
sparq's closure materialization against other reasoners on the LUBM `(ABox+TBox)`
corpus, run `scripts/bench/materialize-same-box.sh` (`ONLY=sparq LUBM_UNIVS=1 …`
for the fast self-check; supply `VLOG=`/`NEMO=` binary paths for those columns).
The oracle is the **closure size** (pinned at `univ=1`: `owl=150589`,
`rdfs=126732`). The VLog and Nemo columns run **validated Datalog encodings**
(`bench/reason-encodings/{vlog/*.dlog,nemo/*.rls}`, `sq-hmd7l.30/.31`) that
reproduce sparq's closure **set-for-set** — so all three engines' closure counts
**AGREE** (folded into `count_crosscheck.same_ruleset_agree`). Critical honesty
caveat: this holds because sparq `reason owl` is the **full W3C OWL 2 RL/RDF** rule
table and the encodings transcribe exactly the rules the LUBM TBox exercises; a
**Jena** column, by contrast, has no full OWL 2 RL reasoner (its `OWL_MICRO`/
`OWL_MINI`/RDFS rule reasoners are OWL-subset + add axiomatic triples) so its
closure size *differs by construction* — recorded per column as a profile caveat,
never reconciled. Never read a raw closure-size delta as a correctness gap without
the profile.

## Gotchas / feature flags / prerequisites

- **Not in the *lean* wasm bundle, but wasm-portable.** `sparq-reason` pulls `regex` and (by default) `rayon`; it is never in the **lean** `sparq-wasm` triplestore bundle. For wasm or single-threaded builds use `default-features = false` (disables the `parallel`/rayon feature). The crate itself compiles to `wasm32-unknown-unknown` — `regex` (the N3 `string:matches` builtin) is pure-Rust and wasm-portable — and ships as the **tier-b `sparq-reason-wasm` ("W-reason") bundle** ([OPUS-4.8] sq-6qw3): a `Reasoner` exposing `materialize` / `entailed` / `materializeStats` / `reasonN3` (and, behind the bundle's opt-in `explain` feature, `why()` / `whyN3()` proof trees — the latter [FABLE-5] sq-ixc3.20: one witness derivation of an N3-derived triple under the same combined rules+facts document `reasonN3` consumes, powering the GUI's click-to-explain proof panel) for in-tab live inference, lazy-loaded on the showcase site's `/surface/inference` page and in the GUI workbench. There is no Noir/ZK toolchain requirement here — proofs are plain Rust structs.
- **Features:** `parallel` (default, rayon-parallel fixpoint), `explain` (NON-default — enables `why()`/`why_with()` and the `explain` module; zero hot-path cost when off, and `why` methods don't exist without it), `d-entail` (NON-default — enables `Profile::D` + the `dtype` module; zero code when off — the lean default/wasm build is byte-identical, `sq-e5atd`), `rif-core` (NON-default — enables the `rif` module: the RIF-Core monotone-Horn rule front-end over the N3 chainer with range-restriction safety; zero code when off, no new `Profile` variant, `sq-rh4gu`), `substrate-join` (NON-default — the RDFS predicate join + the rdfs9 type join + the OWL-RL `Δ⋈full` delta adjacency (`sq-qonbz.2`) drive the SHARED `sparq-substrate::join` kernels; `sq-yk6or` + `sq-pbz04.1.1` + `sq-qonbz.2`, see next bullet), `substrate-compare` (NON-default — the `compare` module: the SHARED `sparq-substrate::compare` SPARQL term total order implemented for dictionary ids, so entailed-solution ordering is parity-identical to the engine's `ORDER BY`; zero code when off, `sq-pbz04.1.2`, see the bullet after next), `compiled-rules` (NON-default — the `n3::compiled` module: id-level COMPILED N3 evaluation for the access-control rule subset over the shared substrate join kernels; zero code when off, `sq-zgbso.3`, see the Key-APIs compiled block).
- **Shared join kernels (`substrate-join`, opt-in, `sq-yk6or`, epic `sq-pbz04`).** The RDFS single-pass predicate join — rdfs7 (subPropertyOf rewrite), rdfs2 (domain typing), rdfs3 (range typing), keyed on the asserted triple's predicate — and the rdfs9 subclass-typing join (`sq-pbz04.1.1`, keyed on the type-assertion's OBJECT column: the "orientation" is just a different `JoinKeys` probe-column index) drive the *same* `sparq_substrate::join::{build_table, probe_emit, hash_probe_serial}` hash-join body the SPARQL engine drives (epic `sq-qonbz` Phase 3, #1300). The reasoner supplies its OWN `JoinKeys` (predicate-keyed) + its OWN `Budget` (the unbounded `NoBudget`; materialisation runs to completion — a closure-level budget is a fixpoint concern, installed around the whole call, not per-join), monomorphically — no `Box<dyn>`/vtable on the probe loop. This is the end-to-end proof of "share join logic across the engine AND the reasoners" (`research/shared-eval-substrate.md` Phase 5). **Behaviour-neutral:** the materialised closure is byte-identical to the hand-rolled `FxHashMap` adjacency path (asserted per-branch by `rdfs::tests::substrate_join_emits_identical_plain_branch` / `substrate_join_emits_identical_type_branch`, and whole-closure by `closure_is_byte_identical_across_join_paths`, which runs in BOTH feature states); only the join machinery changes. **OFF by default** so the byte/bundle ratchets stay exactly the hand-rolled path; the only deps it pulls (`sparq-substrate` `rows`+`join`, `smallvec`) are already in the crate's tree. **Residual disposition (`sq-pbz04.1.1`):** the `PropExpand` inverseOf/Symmetric predicate-rewrite branch is RETAINED hand-rolled *permanently* — its per-match combine is data-dependent (the `swapped` flag picks the subject/object orientation per matched build row) and cascades into a second dom/rng join keyed on the DERIVED predicate, a variable-arity shape the kernel's one-fixed-row-per-match combine cannot express without rebuilding the rule structure around it (full rationale in `substrate_join.rs`; the oriented emission is pinned by `rdfs::tests::prop_expand_inverse_types_through_oriented_domain` so any future adoption attempt inherits a red/green harness). **OWL-RL delta adjacency (`sq-qonbz.2`, NOW SHIPPED under this same feature):** the semi-naive `Δ⋈full` adjacency for `prp-fp` (functional), `prp-ifp` (inverse-functional), and `prp-trp` (transitive) is also behind `substrate-join`. A persistent `DeltaAdj` struct (two `DeltaTable`s — forward `out_tbl` keyed on `[p,s]`, backward `inc_tbl` keyed on `[p,o]`) replaces the per-round nested `FxHashMap` probes; `extend_one` grows both tables incrementally as new delta triples commit, and `probe_out`/`probe_inc` emit results via a generic `FnMut(Id)` closure (monomorphised, no `Box<dyn>`, no vtable — `check-no-dyn-dispatch.py` is clean). **Behaviour-neutral:** the OWL-RL ratchet output is byte-identical in both feature states; the three probe paths (`prp-fp` forward, `prp-ifp` backward, `prp-trp` backward) are pinned by `tests/substrate_join_owl.rs` (8 required-feature tests: fp/ifp/trp alone and in combination, closure-length and no-chain guards). UnionFind (`sameAs` merge) is NOT touched by this change.
- **Shared term total order (`substrate-compare`, opt-in, `sq-pbz04.1.2`, substrate seam 3).** The `compare` module implements the substrate's `CompareTerm` trait for the reasoner's term representation — a dictionary `Id` resolved against its `Dict` (`compare::IdTerm`) — so `compare::compare_ids` / `compare::sort_ids` order ids under the *same* `sparq_substrate::compare::compare_terms` total order the SPARQL engine's `ORDER BY` drives: error/unbound < blank < IRI < literal < RDF 1.2 triple term; literals numeric-aware (with the `exact_cmp` f64-collapse recheck for distinct integers past 2^53), then strict typed/temporal (`xsd:dateTime`/`xsd:date` by TIMELINE via the shared `sparq_core::temporal::Timeline` — cross-timezone order, not lexical; booleans; same-tag language strings; same-other-XSD lexically), then lexical string fallback; triple terms component-wise through the dict's structural component ids. **Ordering parity is pinned byte-for-byte against a REAL engine `ORDER BY`** over the same materialised closure (`tests/compare_parity.rs`, a mixed IRI/bnode/literal/triple-term fixture whose entailed rows participate); the observation hooks reuse the shared machinery (`Timeline`, the substrate `Num`/`Dec` tower, `parse_xsd_f64`) rather than reimplementing it, and the small `Num::of_literal` borrowed-parts mirror is anti-drift-pinned by a unit test against the substrate itself. Adopted MONOMORPHICALLY — `IdTerm` is a generic `CompareTerm` impl, no `Box<dyn>`/`&dyn` between the sort loop and the comparator (`scripts/check-no-dyn-dispatch.py` lists the module). **Purely additive:** no materialiser calls it — which triples are entailed and their emission order are byte-identical in both feature states; undecidable pairs (e.g. `NaN`) collapse to `Equal` exactly as the engine's sort does, and equal-comparing DISTINCT terms (equal values across datatypes, equal instants across timezones) keep stable-sort input order on both sides — the engine's own tie semantics, not a divergence.
- **Two value levels.** RDFS/OWL APIs work on dictionary `Id`s (`materialize*`, `Materialized(Owl)Graph`); N3 batch APIs intern into a `Dict` (`reason_n3`), while term-level N3 (`reason_n3_terms`, `MaterializedN3Graph`) works on `n3::Term` and is **not interned** (formula `{ … }` terms have no dictionary id). Don't mix the two.
- **The materialize → from_parts seam.** `materialize` mutates `(Dict, Vec<[Id;3]>)` *before* indexes are built. Use `Graph::parse_to_triples` (not `Graph::load_str`) so reasoning runs between parse and index build; then `Graph::from_parts`. It interns any vocabulary terms it needs and is idempotent (a second call adds nothing).
- **RDFS scope is deliberate:** the non-explosive subset (rdfs2,3,5,7,9,11 — subClass/subProperty/domain/range). No axiomatic or reflexive `rdfs:subClassOf`/`type` triples (they add no useful inferences and explode the store).
- **D-entailment (`Profile::D`, opt-in `d-entail`) scope + caveats:** materializes the rdfD1 datatype-typing rule — a well-formed literal `"l"^^d` of a *recognized* datatype `d` (the `Recognized` map; `xsd:string`/`rdf:langString` always, `Recognized::standard()` adds the numeric/boolean/temporal core) entails `"l"^^d rdf:type d`. The emitted typing triples are **generalized** (literal in subject position) — feed the closure to a query only after dropping literal-subject rows (they can never be a SPARQL answer; this is also why the W3C `d-ent-01` test correctly returns NO rows). The load-bearing invariant is **value-space equality** via `d_value_eq`: `"1"^^xsd:integer` ≡ `"1.0"^^xsd:decimal` (the integer/decimal value spaces coincide), compared as a CANONICAL DECIMAL STRING — **never an f64 fast path** (f64 silently aliases integers past 2^53 and loses decimal precision). `float`/`double` are a DISJOINT IEEE-754 value space; `date` and `dateTime` are disjoint temporal families. NOTE: the SHARED SPARQL term total order now lives in `sparq-substrate::compare` (`compare_terms` over the generic `CompareTerm` trait — error/unbound < blank < IRI < literal < triple, numeric-aware + strict typed/temporal + string fallback; epic `sq-qonbz` Phase 4, `sq-vezew`, #1300-chain). A reasoner that orders entailed solutions (RIF `order`, an EL/QL `ORDER BY` over a materialised answer set) reuses it by implementing `CompareTerm` for its own term type — the same monomorphisation seam `substrate-join` uses for `JoinKeys`; sparq-reason now SHIPS that impl for dictionary ids behind the opt-in `substrate-compare` feature (`sq-pbz04.1.2`, see the shared-term-total-order bullet above). The trait carries an `exact_cmp` **f64-collapse recheck** hook (`sq-rikm7`): the numeric arm coerces to f64 for speed, and when two operands tie there `exact_cmp` recovers the exact order of distinct integers past 2^53 / high-precision decimals — so a reasoner `ORDER BY` / `MIN` / `MAX` agrees with the relational `=`/`<` rather than falling into the very f64-aliasing this caveat warns about (return `None` from it if your term type has no exact numeric tier). D's typed *value-space-equality* comparator (`d_value_eq`, used for entailment not ordering) stays reasoner-resident for now; D-inconsistency (ill-typed-literal / value-space clashes) and cross-type value-space *subset* reasoning are tracked-not-yet-shipped here (epic `sq-pbz04`).
- **OWL 2 RL is sound but INCOMPLETE for class classification.** Running `Profile::OwlRl` / `--reason owl` over an EL ontology returns a `rdfs:subClassOf` hierarchy that silently omits derivable subsumptions: the RL/RDF rule set's completeness theorem is scoped to *assertional* conclusions, and class classification is a TBox-conclusion task. Two mechanisms, stated precisely — (a) RL has **no TBox conjunction-composition rule**: `B ⊑ C`, `B ⊑ E` ⊬ `B ⊑ C ⊓ E` (`scm-int` only decomposes; `cls-int1` composes only over *individuals*), so `C ⊓ E ⊑ D ⊬ B ⊑ D`; (b) RL never **introduces or reasons through a fresh existential successor**, and `ObjectSomeValuesFrom` in *superclass* position is outside the RL grammar entirely. Caveat worth knowing so you calibrate correctly: sparq implements `scm-svf1/2`, so an existential bridge whose *both* restriction nodes already appear syntactically (`A ⊑ ∃r.B`, `B ⊑ C`, `∃r.C ⊑ D`) **is** in fact derived by this repo's RL — the gap is real but narrower than "RL misses every existential subsumption". Mechanism (a) is the one pinned by an in-repo differential (`crates/sparq-cli/tests/el_cli.rs::el_derives_what_rl_cannot`). For the **complete** class hierarchy use `sparq-reason-el` (above) — via the library, or the CLI's `classify` / `--reason el` under the opt-in `el` feature — not more RL rules.
- **The RL materializer is COMPLETE for the assertion-style RL/RDF rules — the W3C OWL-RL conformance row is at the RL ceiling (sq-350ms).** Every rule with a positive-assertion head in Profiles §4.3 Tables 5/6/9 is implemented (the `owl.rs` per-rule status table + `research/inference-completeness-audit.md` §2/§2b are the per-rule proof). The 13 documented OWL-RL conformance divergences are PROVABLY outside the RL profile, **not** missing rules: TBox-axiom conclusions, invented class expressions (`owl:complementOf`/`unionOf`), reified `owl:AllDifferent` structures, the `prp-pdw`/`prp-fp`/`prp-ifp` **contrapositives** (RL has NO rule producing `owl:differentFrom` between INDIVIDUALS — `dt-diff` emits it only between unequal-value literals, otherwise it appears only in clash bodies), `owl:ReflexiveObjectProperty` (EXCLUDED from the RL grammar — there is no `prp-rfx`), and datatype-range INTERSECTION. They stay documented divergences (closing them would be unsound or beyond-profile); the inference ratchet HOLDS — see `inference-conformance-report.md` and the central scoreboard (`scoreboard::SUITES`, CI job `inference-conformance`) for the current pinned count. Multi-round assertion-rule completeness and the prp-pdw/prp-fp soundness boundary are pinned by in-crate guards in `owl.rs::tests`; the per-divergence disposition pass (sq-pbz04.1.3) re-audited all 13 from the raw export premises/conclusions (verdict: 13/13 PERMANENT, zero in-profile fixes), tagged every report-facing rationale `PERMANENT — …` with its rule-level grounding, and pinned tag+grounding with an in-crate disposition test in `owl_suite.rs`.
- **OWL incremental fallback is silent.** `MaterializedOwlGraph` drops to `OwlMode::Fallback` (re-materializes via `materialize_owl_rl` every mutation, still correct) when the base uses `owl:sameAs`, Functional/InverseFunctional, property chains, restrictions, cardinality, hasKey, oneOf, intersection/union — and on any TBox mutation. Check `.mode()` / `.full_rebuilds()` if incremental cost matters. These usually live in a static TBox, so the mode is decided once at load.
- **N3 incremental qualification is narrow.** `MaterializedN3Graph` only runs `N3Mode::Counting` (truly incremental) for a monotone, input-stratified rule fragment: forward rules with ground-IRI predicates, no conclusion blank nodes, builtins limited to the parity whitelist (`log:uri`, `log:equalTo`/`notEqualTo`, `string:concatenation`/`scrape`/`encodeForUri`), and negation only via the store-scoped `?x log:notIncludes { … }` idiom over input-only predicates. Anything else → `N3Mode::Fallback`; always consult `.fallback_reason()` (`None` ⇔ counting active). The full *batch* N3 engine (`reason_n3`) supports the much larger `math:`/`string:`/`list:`/`time:`/`log:` builtin set and goal-directed `<=` rules.
- **N3 `math:` exact arithmetic is on the SHARED substrate tower (`sq-pbz04.5.1`, seam 2).** The chainer's exact add / subtract / multiply / negate / abs core (and the scale-aligned comparison the max/min and value-equality paths use) DELEGATES to `sparq-substrate::numeric::Dec` — a base, non-optional `numeric`-slice dependency, the SAME exact fixed-point `mant * 10^-scale` decimal the SPARQL engine's FILTER/BIND path drives — so the reasoner and the engine can never diverge on exact-decimal arithmetic (`0.1 + 0.2` is exactly `0.3`; `('2.7' '2') math:difference` is exactly `0.7`). The private `NumVal` enum stays a thin EYE-compat ADAPTER over that core: EYE's own edges are byte-identical and stay adapter-resident — lexical-shape string coercion, the `numval_term` result rendering (whole-`f64` → `xsd:integer`), `math:remainder`'s divisor-sign integer semantics, `math:integerQuotient`'s floor, `math:quotient`'s scale-34 exactness rule (non-terminating → `f64`, exact integer/integer → `xsd:integer`, NOT substrate `Dec::checked_div`'s always-decimal scale-18 rounding), integer `math:exponentiation`, and the `Int`-collapse rendering of floor/ceiling. The i128↔i64 wrinkle: the chainer `Int` tier is `i128` while substrate `Num::Int` is `i64`, so an out-of-`i64`-range integer is carried as substrate `Dec { mant, scale: 0 }` (exact for the full `i128` range; a mantissa overflow falls back to `f64` exactly as before). N3/EYE differential + expressivity floors and `RIF_CORE_FLOOR` are all byte-identical (the closure is unchanged; a direct old-`NumVal`-vs-substrate differential over the `>i64::MAX` / `0.1+0.2` / INF-NaN / `2.7-2` matrix pins it).
- **`reason_n3_pass_all` echoes rules, with two caveats.** `RuleVars::N3` output re-parses to the same rules, so re-running it is a FIXPOINT — *unless* a rule mints an existential (a blank node in its CONCLUSION), which gets a fresh `_:__sk…` label per firing, so such a document grows on every re-run (the same caveat EYE carries). `RuleVars::VarIris` (the `--pass-all-ground` form) replaces `?x` with `<http://www.w3.org/2000/10/swap/var#x>` throughout the echoed RULES — at every depth, quoted `{ … }` formulae included, since N3 quantifies `?x` in the outermost formula — but the rule is then made of CONSTANTS, so feeding that document back to the reasoner does **not** re-derive anything. It grounds rules, not data: a document that ASSERTS a formula-valued fact carrying a variable (`:a :p { ?x :q :b }.`) still echoes that `?x` in the closure half, so "no `?` anywhere" holds for rule documents, not for every input. Parity is by construction (closure + rules), not byte-verified against EYE's own writer: sparq emits full `<…>` IRIs with no `@prefix` reconstruction and sorts the closure statements.
- **`why()` is a witness, not a proof set.** It returns the first derivation in deterministic order, or `None` if the triple isn't in the closure or `ExplainOpts` caps (default depth 128, 65 536 nodes) are exceeded — not an enumeration of all derivations.
- **Deletion semantics:** `delete` removes *base* (asserted) triples; a deleted base triple still derivable from the remainder stays in the closure, and deleting a derived-only fact is a no-op (standard materialized-view semantics).

## Migrating from eye-js (`@sparq-org/eyereasoner-compat`)

The npm package **`@sparq-org/eyereasoner-compat`** (`packages/eyereasoner-compat/`) is a
drop-in for [eye-js](https://github.com/eyereasoner/eye-js)'s `n3reasoner(data, query?, options?)`,
backed by this crate's wasm bundle (`crates/sparq-reason-wasm`) — no SWI-Prolog, a lighter
browser payload. It maps the eye-js surface onto three wasm entry points and is HONEST about the
boundary:

- **Output modes.** `derivations` (default, EYE `--pass-only-new`) → `Reasoner.reasonN3New`;
  `deductive_closure` (`--pass`) → `Reasoner.reasonN3`; `none` → empty. The `…_plus_rules` modes
  (`--pass-all` / `--pass-all-ground`) still **throw in the npm package**: the *reasoner* half
  now exists — `reason_n3_pass_all(src, RuleVars::…)` emits the closure plus the echoed rules
  (`sq-xqchl.2`) — but it is not yet exposed as a `sparq-reason-wasm` entry point, so the
  package has nothing to call. Wiring the binding + the two `switch` arms is the remaining
  step; until it lands the modes fail loudly rather than return a different result set.
- **Query filter.** `n3reasoner(data, query)` evaluates the EYE `--query` rule over the
  materialised closure (`Reasoner.reasonN3Query` → `reason_n3_query`). The premise runs through
  the reasoner's OWN matcher, so **builtins, `{ … }` formulae and `( … )` lists all work** in a
  query rule (`sq-xqchl.1`); the earlier SPARQL-`CONSTRUCT` translation could evaluate none of
  them and rejected them fail-closed.
- **Builtins.** Only this crate's `math:`/`string:`/`list:`/`time:`/`log:` subset is available
  (EYE's full library is larger); an unsupported builtin simply does not fire.
- **SWIPL surface.** `SwiplEye` / `loadEyeImage` / `runQuery` / `EYE_PVM` / `linguareasoner` etc.
  are re-exported as **throwing migration stubs**; `SWIPL` / `cb` options warn-and-ignore.

See `packages/eyereasoner-compat/README.md` for the CDN (esm.sh/jsdelivr/unpkg) usage and the
full output-mode + builtins-coverage tables.

## See also

- `noir-circuit-patterns`, `noir-optimisation`, `verifiable-credentials-zk`, `sparql-formal-semantics` — the single-prover ZK estate; the `explain` `ProofTree` is intentionally a flat, id-free, premises-before-conclusion DAG meant as a ZK-derivation witness.
- `mpc-protocols` — multi-party layer over (federated) SPARQL.
- `hdt-format`, `fused-decompress-parse`, `rust-parallel-parsing` — sibling ingest/storage skills for getting triples into the graph you then reason over.
- `research/owl2-el-ql-reasoning-spike.md` — the EL/QL feasibility spike: why EL first, the RL-incompleteness proof (the CR4 counterexample), and the phased plan (E1–E6) `sparq-reason-el` implements.
- `research/reasoner-suite-on-substrate.md` §2.5 — the QL track design: the PerfectRef applicability trap, the strict CQ-shape gate, and why the production path (tree-witness + UCQ-containment minimisation) is sequenced late by soundness risk (the phased plan `sparq-reason-ql` implements through phases Q1–Q3, and the sparq-extension conformance floors — the DL-Lite_R certain-answer floor `QL_DLLITE_FLOOR` and the sound-subset entailment-arm floor `QL_ENTAILMENT_FLOOR` — have both graduated, sq-qo1a9 / sq-pbz04.3.4).
- `crates/sparq-conformance/tests/ufo_sn3/` — **UFO-SN3**: a finite-world, function-free, range-restricted N3 projection of representative UFO (Unified Foundational Ontology) concepts — rigidity, identity criteria, relators, events/participation, dispositions, commitments/norms, situations/worlds/accessibility — run as committed vocab + rules + fixture cases through plain `reason_n3` (`tests/ufo_sn3_suite.rs`, `UFO_SN3_FLOOR`, an UNGATED sparq-EXTENSION row in the central scoreboard). Demonstrates the reification-node projection for statement-level (triple-term-shaped) claims, since the N3 `Term` model has no triple-term variant (a tracked gap). [FABLE-5]
