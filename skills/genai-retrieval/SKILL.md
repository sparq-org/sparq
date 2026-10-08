---
name: genai-retrieval
description: Use when an LLM/agent needs to answer natural-language questions over a sparq RDF Graph with SPARQL, or to build token-budgeted retrieval/grounding context (schema card, VoID, characteristic-set join hints) about an unknown RDF dataset. Covers the sparq-nlq NL→SPARQL loop (ground→generate→validate→execute→repair, offline record/replay, optional live Anthropic backend or a configurable provider-agnostic OpenAI-compatible endpoint client), its sparq_nlq::eval exec-accuracy harness (answer-set F1, oracle-vs-end-to-end, grounded-vs-ungrounded; live LLM test off by default), and sparq-introspect (schema/VoID/seed-scoped summaries + planner join hints).
---

# sparq genai-retrieval

The AI/agent surface of the sparq RDF+SPARQL engine, in two opt-in, network-free
crates that compose:

- **`sparq-introspect`** — mines the *effective schema* a graph actually uses
  (classes, per-class predicate usage, observed domain/range, characteristic sets,
  cross-class join hints, namespaces) by sorted scans over the store indexes — and
  renders it as a **token-budgeted text "schema card"**, **VoID** (N-Triples),
  **SHACL node shapes** (characteristic sets → `sh:NodeShape`), or full **JSON** for
  LLM grounding / agent retrieval context.
- **`sparq-nlq`** — a deliberately lean **NL→SPARQL loop**: ground (with the
  introspect summary) → generate (an `Llm` behind a trait) → validate (`spargebra`
  parse) → execute (`sparq-engine` under a `QueryBudget`) → repair (≤ N rounds). The
  LLM sits behind a record/replay seam so CI is fully offline; a live Anthropic
  client is behind an opt-in `live` feature.

Both crates are read-only over `sparq-core`'s public API. The engine and core library
do not depend on them (only `sparq-mcp`, `sparq-terse`, `sparq-vectors`, and opt-in
`sparq-server` features do), so the engine carries zero GenAI code.

## Quickstart

`Cargo.toml`:

```toml
[dependencies]
# sparq-introspect and sparq-nlq are publish = false: take all three from one git source.
sparq-core       = { git = "https://github.com/sparq-org/sparq" }
sparq-introspect = { git = "https://github.com/sparq-org/sparq" }
sparq-nlq        = { git = "https://github.com/sparq-org/sparq" }   # add features = ["live"] for the Anthropic backend
```

Add `features = ["schema-diff"]` to `sparq-introspect` when comparing two mined
schemas. The feature is off by default.

Grounding context from a graph (no LLM, no network):

```rust
use sparq_core::Graph;
use sparq_introspect::Introspection;

let graph = Graph::load_str(turtle_or_ntriples, "turtle")?;   // or "ntriples"
let ix = Introspection::build(&graph);

let card = ix.to_text_summary(2500);          // prompt-ready schema card, ≤ 2500 chars
let json = ix.to_json();                       // full machine surface for an agent
let void = ix.to_void("http://ex.org/dataset");// W3C VoID, as N-Triples
let shacl = ix.to_shacl();                      // characteristic sets → SHACL node shapes (N-Triples)
# Ok::<(), String>(())
```

NL→SPARQL, offline via a recorded fixture (the CI path):

```rust
use sparq_core::Graph;
use sparq_nlq::{Nlq, ReplayLlm};

let graph  = Graph::load_str(text, "ntriples")?;
let replay = ReplayLlm::from_file("tests/fixtures/olympics_replay.json")?;
let nlq    = Nlq::new(&graph, Box::new(replay));  // builds the schema summary once

let answer = nlq.ask("How many athletes are on each team?")?;
println!("{}", answer.sparql);                    // the executed query (post-repair)
println!("{} rows, {} repair(s)", answer.result.len(), answer.repairs);
for row in &answer.result.rows {                  // Vec<Vec<Option<oxrdf::Term>>>
    // row[i] is None when var answer.result.vars[i] is unbound
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Key APIs

The internal `sparq-acbench` audience-capability matrix exposes its implemented
GenAI-retrieval catalogue as typed data. Approximate rows are proposals only;
exact graph execution or provenance validates what is answered or cited:

```rust
use sparq_acbench::{AnswerSafety, GENAI_RETRIEVAL_EXPRESSIBILITY};

for row in GENAI_RETRIEVAL_EXPRESSIBILITY {
    println!("{}: {}/{} ({:?})", row.capability, row.provider_crate,
             row.provider_feature, row.answer_safety);
    assert!(matches!(row.answer_safety,
        AnswerSafety::ApproximateProposes | AnswerSafety::ExactValidates));
}
```

<!-- sq-oti9m: public catalogue maintenance note. -->

`sparq-introspect`:

```rust
// Build (one full SPO scan + one POS pass + one dict pass). Default tuning is
// sized for LLM grounding; tune with build_with.
Introspection::build(graph: &Graph) -> Introspection
Introspection::build_with(graph: &Graph, opts: &BuildOptions) -> Introspection
// opts.minimalize_types (default false, sq-lc3): ABSTAT-style type minimalization —
// fold each subject's types to the most-specific via the graph's own rdfs:subClassOf
// chains (transitive closure of in-graph edges; no ontology fetch, no OWL reasoning;
// cycles tolerated as equivalence). A subject typed both dbo:SportsEvent and dbo:Sport
// (SportsEvent ⊑ Sport) is then profiled ONLY under the minimal dbo:SportsEvent.
// Applies uniformly to class extents, per-class usage, CS type histograms, observed
// domain/range, and join hints. Additive (no new dependency); default = full profile.

// Renderers.
Introspection::to_text_summary(&self, budget_chars: usize) -> String   // schema card
Introspection::to_json(&self) -> String                                // pretty JSON
Introspection::to_void(&self, dataset_iri: &str) -> String             // VoID N-Triples
Introspection::to_shacl(&self) -> String                               // CS → SHACL node shapes (N-Triples)
Introspection::schema_summary_for(&self, seeds: &[&str], budget_chars: usize) -> String

// With sparq-introspect feature `schema-diff` (default off), compare two mined
// schemas. Every entry vector is sorted by ascending IRI.
Introspection::diff(&self, other: &Introspection) -> SchemaDiff
// SchemaDiff { triples_base, triples_new, added_classes, removed_classes,
//              changed_classes, added_predicates, removed_predicates,
//              changed_predicates }
// DiffEntry { iri, base, new }; added entries have base=0, removed entries have new=0.
SchemaDiff::to_json(&self) -> String

// Persisted *.introspect sidecar (sq-3n4): mine once, summarise forever — reload is
// O(output), no graph rescan. Format is exactly to_json's (plain JSON).
Introspection::save(&self, path) -> io::Result<()>           // write sidecar (JSON)
Introspection::load(path) -> io::Result<Introspection>       // read+parse (InvalidData on bad JSON)
Introspection::from_json(json: &str) -> serde_json::Result<Introspection>  // in-memory inverse
sparq_introspect::sidecar_path_for(dataset) -> PathBuf       // "g.nt" -> "g.nt.introspect"
sparq_introspect::SIDECAR_EXTENSION: &str                    // "introspect"

// Result fields (all serde Serialize + Deserialize — the sidecar round-trips them):
//   .triples .subjects .entities : u64
//   .classes: Vec<ClassProfile>            // by instance count, per-class predicate usage + coverage
//     ClassPredicate.samples: Vec<String>  // (sq-3n4) per-class sample labels — a minority class
//                                          //  shows ITS OWN values, not the predicate's global min
//   .predicates: Vec<PredicateProfile>     // global stats + inferred/declared domain & range
//   .characteristic_sets: CharacteristicSets
//   .join_hints: JoinHints                 // cross-class (C, p, D) edges + triple counts
//   .vocabularies: Vocabularies            // namespaces + well-known recognition

// Planner-facing: the EXACT characteristic-set table in dictionary-id space (no
// caps, no string resolution). Feeds sparq-engine's `cs-planner` CsTable.
sparq_introspect::characteristic_set_ids(graph: &Graph) -> Vec<CsIdSet>
//   CsIdSet { predicates: Box<[Id]>, subjects: u64, predicate_triples: Box<[u64]> }

// One-call faceted browsing counts. Candidate subjects match the optional class
// and every `(predicate IRI, object N-Triples term)` constraint. Results are ordered
// by count descending then value ascending; `top_k` bounds each distribution.
sparq_introspect::facets(graph: &Graph, request: &FacetRequest) -> FacetResponse
// FacetResponse { candidates, types, predicates, values }
// PredicateValues { predicate, values: Vec<Counted>, elided }
// `facet_predicates: None` computes values for every predicate on the candidates.
// With sparq-introspect feature `numeric-facets` (default off), FacetResponse also has:
//   numeric: Vec<NumericFacet>  // predicate order; predicates without finite numerics omitted
// NumericFacet { predicate, min, max, count, buckets: Vec<NumericBucket> }
// NumericBucket { lo, hi, count }  // ten equal-width buckets; final bucket includes `hi`
// Non-numeric / ill-formed / NaN / infinite objects remain in `values` but are excluded here.
```

`sparq-nlq`:

With the default-off `query-policy` feature, classify generated SPARQL after
parsing its algebra (never by scanning keywords):

```rust
use spargebra::SparqlParser;
use sparq_nlq::policy::{
    classify_query, classify_update, query_form, QueryForm, QueryPolicy,
};

let parser = SparqlParser::new();
let query = parser.parse_query("SELECT * WHERE { ?s ?p ?o }")?;
assert_eq!(classify_query(&query), QueryPolicy::ReadOnly);
assert_eq!(query_form(&query), QueryForm::Select);

let update = parser.parse_update("CLEAR DEFAULT")?;
assert_eq!(classify_update(&update), QueryPolicy::Mutating);
# Ok::<(), spargebra::SparqlSyntaxError>(())
```

```rust
// The single seam to any model — synchronous, stateless (the whole prompt is in `prompt`).
pub trait Llm { fn complete(&self, prompt: &str) -> Result<String, String>; }

// Backends.
ReplayLlm::from_file(path) -> Result<ReplayLlm, String>   // exact-prompt replay (CI)
ReplayLlm::from_json(json) -> Result<ReplayLlm, String>
RecordingLlm::new(inner: L)                               // wraps any Llm; .save(path) writes a fixture
#[cfg(feature = "live")] live::AnthropicLlm::from_env() -> Result<AnthropicLlm, String>
#[cfg(feature = "live")] live::AnthropicLlm::with_model(model) -> Result<AnthropicLlm, String>
// Configurable, provider-agnostic OpenAI-compatible endpoint client (opt-in `nlq-endpoint`;
// base-url + model + optional key are USER-supplied — nothing baked in). sq-2m6zm.6
#[cfg(feature = "nlq-endpoint")] endpoint::EndpointConfig::new(base_url, model)            // .with_api_key/_max_tokens/_timeout
#[cfg(feature = "nlq-endpoint")] endpoint::EndpointConfig::from_env() -> Result<_, String> // SPARQ_NLQ_ENDPOINT_{URL,MODEL,KEY}
#[cfg(feature = "nlq-endpoint")] endpoint::EndpointLlm::new(EndpointConfig) / ::from_env() -> Result<EndpointLlm, String>

// The loop.
Nlq::new(graph: &Graph, llm: Box<dyn Llm>) -> Nlq
Nlq::with_config(graph: &Graph, llm: Box<dyn Llm>, config: NlqConfig) -> Nlq
Nlq::ask(&self, question: &str) -> Result<Answer, NlqError>
Nlq::prompt_for(&self, question: &str) -> String          // deterministic; for building fixtures
Nlq::repair_prompt_for(&self, question, failed_sparql, error) -> String

// Index-grounded entity/relation linking (sparq-sim) — opt-in via NlqConfig.link_entities.
// Resolves the question's proper nouns to concrete IRIs in THIS store's dictionary and
// surfaces them (with structurally-similar siblings from sparq_sim::Sim::most_similar) in
// the grounding prompt. Usable standalone:
sparq_nlq::link::EntityLinker::build(graph: &Graph, expand_k: usize, max_links: usize)
EntityLinker::link(&self, question: &str) -> Linking
// link() tries a verbatim FULL-LABEL match first (case/whitespace-normalised, punctuation
// preserved): when the whole input IS an entire label it binds that one entity exactly
// (sq-26fdp), so a verbatim prefLabel never goes ambiguous; otherwise the token n-gram path runs.
// Linking { entities: Vec<LinkedEntity>, relations: Vec<LinkedRelation>, values: Vec<LinkedValue> }
//   LinkedEntity { mention, iri: NamedNode, label, exact: bool, similar: Vec<NamedNode> }
//   LinkedRelation { mention, iri: NamedNode, triples: u64 }
//   LinkedValue { mention, literal: Literal, predicates: Vec<NamedNode>, uses: u64 }
Linking::to_prompt_section(&self) -> Option<String>      // None when nothing linked

// EXACT dictionary linking (sq-na0q) — opt-in via NlqConfig.link_values, or standalone:
EntityLinker::build(graph, expand_k, max_links).with_values()   // one object-sorted scan
// Adds the lexical/EXACT tier the schema card and the structural sim cannot give you:
//  * a question span that IS, verbatim (up to case), the lexical form of a literal the
//    store holds resolves to that literal with its DATATYPE and LANGUAGE TAG intact
//    ("1994"^^xsd:gYear, "France"@en) plus the predicates it is the object of — so a
//    FILTER/value-bound query stops guessing a lexical form that matches no triple;
//  * an IRI written out verbatim in the question is probed straight through
//    Graph::id_of, so it binds exactly even when the entity carries NO label.
// Rendered as its own "# Values from the question found in THIS dataset" prompt block, so
// the IRI block stays byte-identical when nothing value-linked (fixtures keep replaying).
// Values.is_empty() unless with_values() was called. A literal that is merely the label of
// an already-linked entity is dropped (the IRI is strictly better than a label join).
// BOUNDED, on purpose: literals longer than 96 chars are not indexed and the index holds
// at most 100_000 distinct literals in object-id order — beyond that, coverage is that
// prefix, not the whole store.

// Answer { sparql: String, result: QueryResult, repairs: usize, transcript: Vec<Turn> }
// NlqError { message: String, transcript: Vec<Turn> }  (impls Error/Display)
// QueryResult { vars: Vec<Variable>, rows: Vec<Vec<Option<Term>>> } — re-exported by sparq-nlq.
```

`NlqConfig` knobs (`Default` = the config the committed fixtures were recorded under):
`summary_budget_chars` (4000), `max_repair_rounds` (1), `exec_timeout`
(`Some(10s)`, native only), `max_rows` (`Some(1_000_000)`), `examples`
(two schema-agnostic few-shots), `ground` (`true` — the grounded loop the fixtures
were recorded under; set `false` for the ungrounded baseline the exec-accuracy
harness measures grounding against — see below), `link_entities` (`false` — opt-in
sparq-sim entity/relation linking appended to the grounded prompt; flipping it on a
recorded dataset means re-recording, since the prompt string changes), `link_expand_k`
(`3` — structurally-similar siblings per linked entity), `max_links` (`8`), `link_values`
(`false` — the **exact dictionary** tier of `sq-na0q`: literal values and verbatim IRIs
from the question resolved against the store's dictionary and fed into the prompt with
datatype/language tag intact; read only when `link_entities` is also on, and it too changes
the prompt string), and
`check_dictionary` (`false` — the **N2 dictionary constraint**, bead `sq-9yjp`;
see below), and `guard` (`GuardConfig::default()` — injection/budget hardening, **on**;
see below).

**N2 dictionary constraint** (`sparq_nlq::constrain`, opt-in `check_dictionary`):
after a query parses, walk its algebra and check every predicate / `rdf:type` class
IRI against the live dictionary (`Graph::id_of`). An ungrounded IRI matches no triple,
so it becomes a targeted repair signal (`TurnOutcome::UngroundedTerms`) with
nearest-namespace did-you-mean suggestions pulled from the store. Off by default: an
empty-answer question (a valid-but-absent class) must not be "repaired", and the
exec-accuracy harness measures raw model accuracy with it off. Strict logit-level
**grammar-constrained decoding** is *not* implemented — it needs logit/grammar access
the Anthropic Messages API does not expose, so it is only feasible on a local backend
(`research/genai-nl-to-sparql.md` §11). Public surface:
`constrain::unknown_terms(graph, &spargebra::Query) -> Vec<UnknownTerm>`,
`constrain::dictionary_repair_message(&[UnknownTerm]) -> String`,
`UnknownTerm { iri, role: TermRole, suggestions }`.

**Injection + budget hardening** (`sparq_nlq::guard`, **on by default**, no feature —
bead `sq-j1wv`, threat model `research/nlq-threat-model.md`). The loop splices three
untrusted strings into a prompt (the question; graph-derived text — entity labels and
schema-summary samples; the model's own echoed output) and then executes what comes
back. Two layers:

- *Input side (mechanical, best-effort)*: `guard::flatten_untrusted` (question, linked
  labels/mentions) neutralizes 3+-backtick fences and folds control characters/line
  separators to spaces, so untrusted text cannot open a fence or forge a `Question:`
  line; `guard::sanitize_block` does the same for the schema summary but **keeps `\n`**
  (its line structure is load-bearing); `guard::check_question` rejects a question over
  `max_question_chars` **before any LLM call**; both repair-prompt echoes are capped at
  `max_echo_chars` with an explicit `[truncated N characters]` marker.
- *Output side (the containment that matters)*: `guard::forbidden_constructs` walks the
  parsed algebra and refuses `SERVICE` federation — the exfiltration/SSRF payload —
  unless `allow_federation` is set, yielding `TurnOutcome::Forbidden(Vec<Forbidden>)`
  as a repair signal **before execution**. Mutation needs no check: the loop parses with
  `parse_query`, so an `INSERT`/`DROP`/`LOAD` is a syntax error, not a blocked operation.

Every transform is a **no-op on benign text** (returns `Cow::Borrowed`), so recorded
fixtures still replay byte-identically. **Honest scope:** this bounds an injection's
*consequences*; it does **not** prevent a model being persuaded to answer a different
question, and no such claim is made. Public surface:
`guard::GuardConfig { max_question_chars (4096), max_echo_chars (8192), allow_federation (false) }`,
`guard::check_question(&str, &GuardConfig) -> Result<Cow<str>, GuardError>`,
`guard::forbidden_constructs(&spargebra::Query, &GuardConfig) -> Vec<Forbidden>`,
`guard::forbidden_repair_message(&[Forbidden]) -> String`,
`guard::{neutralize_fences, flatten_untrusted, sanitize_block}(&str) -> Cow<str>`,
`guard::truncate_untrusted(&str, max) -> Cow<str>`,
`Forbidden::Federation { endpoint }` + `::message()`, `GuardError::QuestionTooLong { chars, max }`.

`sparq_nlq::eval` — the **exec-accuracy harness** (the design doc's accuracy gate,
`research/genai-design.md` §4). It grades the *executed* query the QALD way — **answer-set
F1**, not query-string equality — and is **fully offline**: it drives `Nlq` over any
`Llm` (in CI a `ReplayLlm` fixture or a scripted in-memory backend); nothing here
touches the network.

```rust
use sparq_nlq::eval::{self, EvalCase, Linking, Comparison, Report};

// One case = a question + the GOLD SPARQL. The gold is executed on the SAME graph at
// scoring time (no checked-in answer blob to drift — research/genai-nl-to-sparql.md §4.3).
EvalCase::new(question: impl Into<String>, gold_sparql: impl Into<String>) -> EvalCase

// Score one (linking, grounding) configuration over the cases against `graph`.
// For Linking::Oracle the gold query is fed straight through validate→execute and the
// `llm` is NOT consulted (isolates the engine-side loop from model linking).
eval::run_config(graph: &Graph, cases: &[EvalCase], llm: Box<dyn Llm>,
                 config: NlqConfig, linking: Linking) -> Report

// Drives all four cells (end-to-end | oracle) × (grounded | ungrounded). `make_llm(ground)`
// is called once per end-to-end config (ReplayLlm is consumed by the loop, so hand out a
// fresh fixture each call; the grounded/ungrounded prompts are distinct strings).
eval::run_comparison(graph: &Graph, cases: &[EvalCase], base_config: &NlqConfig,
                     make_llm: impl FnMut(bool) -> Box<dyn Llm>) -> Comparison

// Linking ∈ { EndToEnd, Oracle } — reported SEPARATELY, per the design doc.
// Report { linking, grounded: bool, cases: Vec<CaseResult>, macro_f1, exact_match,
//          validity, total_repairs }  // macro_f1 = QALD headline metric
// CaseResult { question, outcome: Result<CaseScore, String> }  // Err = loop failure (F1=0 vs non-empty gold)
// CaseScore { f1: F1, sparql, repairs }
// F1 { precision, recall, f1, intersection, predicted, gold } ; F1::is_exact() -> bool
// AnswerSet::from_result(&QueryResult) -> AnswerSet  // order-independent multiset of rows; F1::score(pred, gold)
// Comparison { grounded_end_to_end, ungrounded_end_to_end, grounded_oracle, ungrounded_oracle }
// Comparison::headline_grounding_pays() -> bool  // grounded end-to-end macro-F1 > ungrounded ("grounding pays for itself")
// Comparison::summary() -> String                // a stderr table; printed at run time, no perf numbers baked into docs
```

## Common recipes

**1. One-call grounded retrieval context for an agent.** Hand the schema card plus
the seed-scoped detail into your own prompt:

```rust
let ix = Introspection::build(&graph);
let overview = ix.to_text_summary(3000);                  // dataset-wide card
let detail   = ix.schema_summary_for(                     // zoom into entities you care about
    &["http://xmlns.com/foaf/0.1/Person", "http://dbpedia.org/ontology/team"],
    1500,
);
let context = format!("{overview}\n\n{detail}");          // feed to your LLM
```

**2. Record a live session, then replay it forever.** Run live once to capture a
fixture; CI replays it with no network:

```rust
# #[cfg(feature = "live")] {
use std::rc::Rc;
use sparq_nlq::{live::AnthropicLlm, Nlq, NlqConfig, RecordingLlm};

let llm = Rc::new(RecordingLlm::new(AnthropicLlm::from_env()?));  // ANTHROPIC_API_KEY
let nlq = Nlq::with_config(&graph, Box::new(Rc::clone(&llm)), NlqConfig {
    max_repair_rounds: 3,                 // design-doc cap for live runs
    ..NlqConfig::default()
});
let _ = nlq.ask("Which team has the most athletes?")?;
llm.save("session.json")?;                // a ReplayLlm fixture for the regression set
# }
# Ok::<(), Box<dyn std::error::Error>>(())
```

**3. Inspect the provenance / debug a failure.** Every `ask` carries the full
transcript; `NlqError` carries it too:

```rust
match nlq.ask("a hard question") {
    Ok(a) => for t in &a.transcript {
        // t.prompt, t.completion, t.sparql: Option<String>, t.outcome: TurnOutcome
        // TurnOutcome ∈ { Ok{rows}, NoSparql, ParseError(s), ExecError(s) }
        println!("{:?}", t.outcome);
    },
    Err(e) => eprintln!("{e}\n{:#?}", e.transcript),  // Display = "<message> (after N turn(s))"
}
```

**3b. Cite an answer from provenance (`--features citations`, default off).** When the
graph carries PROV-O provenance (e.g. the PKG's Findings with `prov:wasDerivedFrom` +
`dcterms:source` + `pkg:confidence`/`pkg:assurance`), `Answer::citations(&graph)` resolves
each answer-row binding to a numbered footnote. Citations are **emitted from provenance,
never generated** — every one resolves to a real in-graph source (resolution rate 1.0,
zero fabricated refs), and a binding with no provenance is reported as "no source
recorded", never guessed. sq-2489d.1

```rust
let answer = nlq.ask("which findings are about the merge discipline?")?;
let cited = answer.citations(&graph);            // feature = "citations"
for c in &cited.citations {
    println!("{}", c.footnote());                // "[1] <source> — anchor (Claimed, conf=0.98)"
}
assert_eq!(cited.resolution_rate(&graph), 1.0);  // every citation resolves in-graph
println!("{}", cited.footnotes());               // block, incl. any "no source recorded"
// The cross-cutting primitive is also callable directly:
//   sparq_nlq::provenance::join(&graph, &answer.result) -> Vec<ProvenanceRecord>
//   sparq_nlq::provenance::provenance_for(&graph, &subject) -> ProvenanceRecord
```

**3c. Qualify / hedge / abstain on an answer (`--features citations`, default off).** The
same Phase-1 provenance join also drives **answer-qualification**: fold the answer's
supporting `pkg:assurance`/`pkg:confidence` **weakest-link** into a verb hedge + verbal
band, and below a `min_confidence` floor **abstain**. The knob lives on `NlqConfig::qualify`
and `Nlq::ask_qualified` returns the `Answer` plus an `AnswerQualification`. The band
*reflects asserted assurance* — it is **NOT** a calibrated confidence; no reliability
measurement exists yet, and none is claimed. sq-2489d.2

```rust
use sparq_nlq::{NlqConfig, qualify::QualifyConfig};
let cfg = NlqConfig {                               // feature = "citations"
    qualify: Some(QualifyConfig { min_confidence: Some(0.5), ..Default::default() }),
    ..NlqConfig::default()
};
let nlq = sparq_nlq::Nlq::with_config(&graph, llm, cfg);
let qa = nlq.ask_qualified("is ZK soundness externally signed off?")?;
println!("{}", qa.qualification.phrase());          // "appears to be (claimed) [confidence: …]"
if qa.qualification.abstain { /* "insufficient confidence to answer" */ }
// Weakest-link: min supporting confidence drives the band; weakest assurance drives the
// verb hedge; a binding with no provenance contributes the weakest rung (Missing), so it
// can only pull toward hedging — never toward over-confidence. Post-hoc + read-only: the
// query, execution and transcript are unchanged (so fixtures need no re-record).
```

**4. Export VoID for a dataset catalog / DCAT entry.** Output is N-Triples (a Turtle
subset), so it re-parses with either parser:

```rust
let void_nt = Introspection::build(&graph).to_void("http://ex.org/dataset");
// void:Dataset with void:triples / void:entities / void:distinctSubjects /
// void:classes / void:properties, plus one classPartition per class and one
// propertyPartition per predicate. NOTE: void:distinctObjects is NOT emitted.
```

**4b. Bootstrap SHACL shapes from the effective schema.** Each characteristic set becomes
an `sh:NodeShape`; output is N-Triples (valid Turtle):

```rust
let shapes_nt = Introspection::build(&graph).to_shacl();
// Per characteristic set: a sh:NodeShape with sh:targetClass for every class UNIVERSAL
// to the set (count == subjects), one sh:PropertyShape (sh:path + sh:minCount 1) per
// non-type predicate, and sh:maxCount 1 only when that predicate is single-valued across
// the set (avg multiplicity 1). Constraints are mined from what the data asserts, so the
// graph already validates against them — a data-grounded floor to edit, not a contract.
// Sets with no universal class yield a reusable but target-less shape.
```

**5. Tune the introspection for a huge or noisy KG.** Bound the histograms and tables:

```rust
use sparq_introspect::{Introspection, BuildOptions};
let opts = BuildOptions { max_char_sets: 200, max_namespaces: 50, max_join_hints: 200,
                          ..BuildOptions::default() };
let ix = Introspection::build_with(&graph, &opts);   // tails aggregate into elided_* fields
```

**6. Feed exact characteristic sets to the engine's cardinality planner.** Requires
`sparq-engine` built with `--features cs-planner`:

```rust
# #[cfg(feature = "cs-planner")] {
use sparq_engine::cs::{CsSet, CsTable};
let table = CsTable::new(
    sparq_introspect::characteristic_set_ids(&graph).into_iter().map(|s| CsSet {
        predicates: s.predicates, subjects: s.subjects, predicate_triples: s.predicate_triples,
    }),
);
// ids are valid only against THIS graph's dictionary — rebuild when the graph rebuilds.
# }
```

**7. Measure NL→SPARQL exec-accuracy (answer-F1) offline.** Build a small set of
`(question, gold SPARQL)` cases, then run all four cells and assert that grounding pays:

```rust
use sparq_nlq::eval::{run_comparison, EvalCase};
use sparq_nlq::{NlqConfig, ReplayLlm};

let cases = vec![
    EvalCase::new("How many athletes are on each team?",
                  "SELECT ?t (COUNT(?a) AS ?n) WHERE { ?a :team ?t } GROUP BY ?t"),
    // … more (question, GOLD query) pairs
];
// `ground` flips per cell; the closure hands out a fresh fixture per end-to-end call.
let cmp = run_comparison(&graph, &cases, &NlqConfig::default(), |ground| {
    let f = if ground { "tests/fixtures/grounded.json" } else { "tests/fixtures/ungrounded.json" };
    Box::new(ReplayLlm::from_file(f).unwrap())
});
eprintln!("{}", cmp.summary());                 // printed at run time only
assert!(cmp.headline_grounding_pays());          // grounded end-to-end macro-F1 > ungrounded
# Ok::<(), Box<dyn std::error::Error>>(())
```

What this proves (and what it does NOT): with scripted/recorded completions it validates
the **harness** and the **mechanism** of the headline claim — answer-F1 scoring,
oracle-vs-end-to-end and grounded-vs-ungrounded reporting, the inequality — **not** a
real model's accuracy numbers. Those come from a `live_exec_accuracy` run (`--features
live` + `RecordingLlm`, which needs a key + network + dataset); that test is `#[ignore]`'d
and feature-gated **OFF**, so a model or network outage can never red the build. After a
live run the recorded sessions become the offline regression set — and the
record→`save`→`from_file`→identical-scores round-trip that makes them a *faithful*
regression set is itself CI-gated (`recorded_session_saves_and_replays_as_regression_set`
in `tests/exec_accuracy.rs`). (No claim is made here about NL→SPARQL quality beyond what
the fixtures encode.)

## sparq-terse — the verifiable LLM-ergonomic query surface (opt-in)

`sparq-terse` (epic `sq-2m6zm`, design `research/llm-ergonomic-sparql-surface.md`,
PR #1074) is a **pre-parse transpiler** layered over this stack: it lets an agent write the
*concept it means* instead of guessing an opaque IRI, while only ever executing canonical,
conformant SPARQL. It **never** touches the vendored `spargebra` grammar — the engine sees
standard SPARQL it can inspect ("a convenience that shows its work, not an oracle").

```rust,ignore
use sparq_terse::{terse_to_sparql, terse_to_sparql_with, ResolveCtx, Method, legend_card};
use sparq_core::Graph;

// Phase 1 (default build, lean: only spargebra). Canonical SPARQL passes through
// byte-identical AFTER the silent-rewrite canary re-parses the emission; a non-parsing
// output is TerseError::CanaryFailed, never handed back.
let exp = terse_to_sparql("SELECT ?s WHERE { ?s <http://ex/p> ?o }")?;
assert_eq!(exp.canonical_sparql, "SELECT ?s WHERE { ?s <http://ex/p> ?o }");

// Phase 3 / lever 1 (default build). A K:<name> keyword expands pre-parse to its frozen
// IRI (no PREFIX line); the expansion is echoed in exp.keywords. An unknown keyword, or a
// clash with a real `PREFIX K:`, is a HARD error (never a guess). Publish the frozen
// legend ONCE behind the prompt-cache breakpoint with legend_card() — the token win is a
// CACHING property (design §1.6), not query-body terseness.
let exp = terse_to_sparql("SELECT ?f WHERE { ?f K:type K:Finding ; K:derivedFrom ?s }")?;
assert!(exp.canonical_sparql.contains("<http://www.w3.org/ns/prov#wasDerivedFrom>"));

// Phase 2 (feature = "vectors"). V("phrase") concept resolution, lexical-FIRST.
let graph = Graph::load_str(turtle, "turtle")?;
let ctx = ResolveCtx::lexical(&graph);                 // no model, no network (the default)
let exp = terse_to_sparql_with(
    "SELECT ?f WHERE { ?f <http://ex/about> V(\"cardinality estimation\") }",
    &ctx,
    |_phrase| None,                                    // embedder: vector fallback only
)?;
for r in &exp.resolutions {                            // every bind is echoed for the agent
    println!("V(\"{}\") -> <{}>  score {:.3} conf {:.3} via {}",
             r.phrase, r.iri, r.score, r.confidence, r.method.as_str());
}
# Ok::<(), sparq_terse::TerseError>(())
```

The **`K:<name>` keyword layer** (lever 1, default build, design §3.1, sq-vfeme) is the
lean, no-model half: a small, **frozen, versioned** legend (`LEGEND_VERSION` =
`pkg-keywords/v1`) of the PKG hot predicates/classes (`K:type`/`label`/`subClassOf`/
`derivedFrom`/`generatedBy`/`about`/`confidence`/`dependsOn`/`status`/…), expanded pre-parse
to `<iri>`. It is in the **default build** — independent of the `vectors` feature, and it
composes with `V()` through `terse_to_sparql_with`. Guardrails: an unknown `K:<name>` is
`TerseError::UnknownKeyword` (with did-you-mean, never auto-applied — that would be the
lever-2 anti-pattern); a `K:<name>` clashing with a real `PREFIX K:` is
`TerseError::KeywordPrefixCollision`. Every expansion is echoed in `Expansion.keywords`.
The token win is a **caching** property — place `legend_card()` once behind the prompt-cache
breakpoint, do not re-bill it per turn.

The **§6 soundness envelope** is enforced, all opt-in and none silent: always-canonical
output (the silent-rewrite canary); echo IRI + score + runner-up + confidence + method;
**confidence-gated** (below the floor or inside the ambiguity margin `V()` returns
`TerseError::Unresolved` with candidates — loud-fail beats silent-wrong, never auto-accept
the uncertain); **lexical-first, vector-fallback** (the `sparq-nlq` lexical linker is
primary; the staleness-guarded `sparq-vectors` search is the fuzzy fallback); and a
**mandatory staleness guard** (a store built against a different graph generation is a hard
`TerseError::StaleStore`). It reuses, not reinvents: `sparq-nlq::link::EntityLinker`
(lexical) + `VectorStore::check_graph` + `ann::nearest_exact` (vector).

**Adoption verdict (MEASURED, non-sycophantic — bead `sq-bzign`, the adoption gate, PR #1174).**
A pre-registered, falsifiable query-authoring A/B over the real PKG (30 frozen stratified tasks,
deterministic blind grading by the real transpiler + engine; record in `bench/terse/RESULTS.md`,
numbers are work-box / NON-CANONICAL) gives a **per-lever** result, NOT a blanket win:

- **Lever 1 (`K:<name>` keywords): conditional adopt.** Clears the cache-discounted token bar
  *and* ties plain SPARQL on quality. Conditional only because the headline win is a *caching*
  property, pending a full-session real-transcript fan-out (`sq-bmpzd`).
- **Lever 3 (`V("phrase")`): verbatim labels now resolve; still check, not trust.** The
  original A/B loud-failed on a punctuation-heavy verbatim `prefLabel`
  (`"ZK/MPC claim + circuit discipline"`) because the token-overlap linker split it on the
  shared token `discipline` and (correctly, by the envelope) refused to guess between two
  equally-scored topics. `sq-26fdp` fixes that: the linker now tries an **exact, case- and
  whitespace-normalised, punctuation-preserving full-label match FIRST**, so a phrase that IS
  an entire label binds unambiguously at score 1.0 before token-overlap runs; re-running the
  A/B restores lever-3 `resolution_correctness` to **1.0**. The soundness envelope is
  unchanged for genuinely fuzzy phrases (a phrase that is *not* a whole label and merely
  shares tokens with several concepts still loud-fails). `V()` remains a convenience that must
  be checked (echoed IRI + score + confidence), not trusted blind; prefer `K:` + explicit
  `<iri>` for anything load-bearing.

## sparq-vectors `compose` — read-path URI-hiding view (opt-in, `compose` feature)

The **read-path** sibling of the terse write-path: where `K:`/`V()` help the agent *author* a
query, `sparq_vectors::compose` decides how an answer-row binding is *presented back*. It takes
the IRIs the exact engine produced and renders each one under one of two views — `View::UriVisible`
(the raw IRI, the NL→SPARQL incumbent) or `View::UriHidden` (a label/grounded view + a
`ResolutionEcho`, so the IRI never reaches the agent). It reuses the `verbalize` label machinery
(the feature implies `structure`), adds no new dependency, calls no model, and is **off the default
build** (`compose` feature). sq-mztg8.1, design `research/fo-llm-bridge.md` §2.3/§3.3/§6.

```rust
use sparq_vectors::compose::{ab_report, compose, ComposeConfig, View};

let cfg = ComposeConfig::default();                              // min_confidence 0.5
let hidden = compose(&graph, &bindings, View::UriHidden, &cfg);  // labels + echo, never the IRI
for b in &hidden {
    if let Some(e) = &b.echo {                                   // resolution is always echoed
        println!("{}  via {:?} conf {:.1}", b.shown, e.method, e.confidence);
    }
}
let report = ab_report(&graph, &bindings, &cfg);                 // the model-free A/B
assert!(report.fidelity_preserved());                            // soundness gate (no collisions)
# Ok::<(), Box<dyn std::error::Error>>(())
```

It resolves **lexical-first** (a real `rdfs:label`/`skos:prefLabel` → `ResolutionMethod::Lexical`,
confidence 1.0), falls back to the IRI local name (`LocalName`, 0.5), and **falls open to the raw
IRI** (`None`, 0.0) below `min_confidence` — hiding never costs the agent the node's identity. The
soundness gate is `AbReport::fidelity_preserved()`: if two distinct answer IRIs collapse onto one
shown label the hide is lossy (it would silently change the answer), so the harness counts the
collisions and the bench example exits non-zero. This is the same envelope discipline as the terse
surface: a convenience that always shows its work, never an oracle.

**Honest scope — a legibility + (conditional) accuracy tool, NOT a token-saver.** `compose` and
`ab_report` measure the two model-free properties: round-trip *fidelity* (is the hidden view safe?)
and *presentation cost* (`AbReport::char_ratio`, chars the agent reads). The headline accuracy
question (open question **K4** — does hiding actually help a real model answer?) is now **MEASURED**
(sq-3pb7f): a real-model, blinded, paired NL→answer A/B over the real PKG found the hidden view
**matches-or-beats** raw IRIs on every question and **never regresses one** (a significant overall
gain). But the win is **conditional, not universal** — it is concentrated on bindings whose **IRI
local name is an opaque id** (e.g. `kb:task-sq-XXXX` task nodes, where the raw arm scores ~zero and
the label rescues the answer) and is **~zero where the slug is already informative** (e.g.
`topic-merge-discipline`, `surface-genai-retrieval`, which the agent reconstructs from the local
name). So the right mental model is "**hiding rescues opaque-id bindings**," not "labels beat IRIs
everywhere." And it is still **not a token-saver**: the model-free A/B shows the hidden view is sound
(zero collisions, full coverage) but **larger to read, not smaller** — PKG task/finding labels are
long descriptions, so `char_ratio` runs *above* 1.0 overall, and the very class that drives the
accuracy gain (long task titles) is the one that inflates the read cost — hiding buys accuracy on
opaque ids *with* tokens, it does not save them. The measured figures + the per-kind breakdown live
in `bench/compose/RESULTS.md` and `bench/compose/k4/` (work-box, NON-CANONICAL).

## Gotchas / feature flags / prerequisites

- **Opt-in crates.** No default engine/server feature pulls either crate (`sparq-introspect`
  is a workspace `default-members` entry; `sparq-nlq` is not); add `sparq-introspect` /
  `sparq-nlq` explicitly. They depend only on `sparq-core`'s public scan API, so they
  work against every storage mode (raw, mmap'd, compressed).
- **`live` feature (sparq-nlq).** OFF by default — without it there is no
  `live::AnthropicLlm` and no network code. With it, `AnthropicLlm` reads
  `ANTHROPIC_API_KEY`, POSTs `https://api.anthropic.com/v1/messages` (blocking, 120 s
  timeout), default model `claude-sonnet-4-6` (configurable via `with_model`). CI
  only compile-checks this path; no test calls the network.
- **`nlq-endpoint` feature (sparq-nlq, PRODUCT flavor, sq-2m6zm.6).** OFF by default. With
  it, `endpoint::EndpointLlm` is a **provider-agnostic** OpenAI-compatible chat-completions
  client: the base URL, model id, and (optional) key are **entirely user-supplied** (args
  or `SPARQ_NLQ_ENDPOINT_{URL,MODEL,KEY}` env) — NOTHING is baked in, and it never phones
  home. It POSTs `{base_url}/chat/completions` (blocking, 120 s default timeout) sending
  the prompt as one user message; a key, when present, goes as `Authorization: Bearer`.
  This productizes the measured agent-flavor cost win (sq-zbyo7) for **external** users who
  bring their own cheap-model endpoint (Ollama / vLLM / llama.cpp / OpenAI / …). **Answer
  quality depends ENTIRELY on the user-chosen model** — this crate makes no accuracy claim.
  Unlike `live`, the loopback-stub tests exercise the real request/parse path **offline**
  (no live network), so this lane gates in the feature matrix.
- **`citations` feature (sparq-nlq).** OFF by default — without it the `cite` /
  `provenance` modules and `Answer::citations` are not compiled, and the lean loop carries
  zero provenance machinery. The renderer is read-only and **never invents a citation**:
  it emits only `prov:wasDerivedFrom` sources the graph actually asserts (so resolution
  rate is 1.0 by construction), and reports un-sourced bindings as "no source recorded".
  It works on any PROV-O-annotated graph; the PKG is the lowest-friction case.
- **`citations` feature ALSO gates answer-qualification** (`qualify` module,
  `Nlq::ask_qualified`, `NlqConfig::qualify`). Reuses the SAME provenance join — no second
  machinery. Weakest-link by design: `min` confidence + weakest assurance, so an extra
  low/un-provenanced binding can only lower the qualification. The verbal band **reflects
  asserted assurance — it is NOT a calibrated confidence**; a "calibrated confidence" claim
  needs a reliability-diagram measurement on calibrated confidences, which does not exist
  yet (deferred, `research/provenance-driven-genai-kb.md` §5 Phase 2). Qualification is
  post-hoc + read-only (the prompt/query/transcript are unchanged), so it is
  fixture-compatible — flipping it on needs no re-record. sq-2489d.2
- **`ReplayLlm` is exact-prompt match.** Prompts are deterministic functions of the
  graph + `NlqConfig` + examples, so a miss means the prompt template, the default
  `NlqConfig`, or the dataset drifted — re-record (e.g.
  `cargo run -p sparq-nlq --example record_olympics --release`). The miss error
  surfaces the question lines that *are* recorded.
- **Generated SPARQL is untrusted.** `ask` always executes under a `QueryBudget`
  (default 10 s wall clock + 1M rows). Budget trips surface as `TurnOutcome::ExecError`
  and are a repair signal. Opt out only by setting `exec_timeout` / `max_rows` to
  `None` explicitly. `exec_timeout` is native-only (no `Instant` on `wasm32`; the
  deadline is simply not set there).
- **Prompt constrains the model** to SELECT/ASK, schema-summary vocabulary only, no
  property paths, no federation, one ```sparql block. `extract_sparql` tolerates a
  ```sparql fence, a bare ``` fence, or bare query text starting `PREFIX`/`SELECT`/`ASK`.
- **ASK answers** use the engine's unit-row encoding: zero `vars`, one empty row iff
  true (`answer.result.vars.is_empty()`, `result.len() == 1` when true).
- **`eval` (exec-accuracy) is offline by construction and grades the answer set, not
  the query string.** It executes both the candidate and the **gold** query on the same
  graph and compares bind-row multisets (order-independent; empty-vs-empty scores 1, the
  QALD true-negative convention). The *live-model* accuracy run (`live_exec_accuracy`) is
  `#[cfg(feature = "live")]` **and** `#[ignore]`'d — it is never in the gate, so the
  scored numbers in CI come only from scripted/recorded completions. Treat them as a
  harness/mechanism check, not a model-quality benchmark.
- **Summary hints are hints.** In `to_text_summary`, per-class counts/coverage are
  exact, but the trailing `→ range, e.g. sample` come from the predicate's *global*
  profile. `schema_summary_for` is struct-level scoping (filters already-mined
  profiles by IRI) — it does NOT re-scan, so it won't chase a seed entity's instances.
- **Cost.** Introspection is `O(|G| + |dict|)` (sorted scans, no GROUP BY). LLM-excluded
  `ask` latency is the engine's query time — the LLM round trip typically dominates wall clock.
- **Graph scoping (sq-quuu).** `Introspection::build(&graph)` describes the **store of
  the `Graph` it is handed** — the **default graph** for the top-level `&graph`, or a
  **single named graph** when you pass that graph's sub-`Graph`. A schema card / VoID is
  always scoped to exactly one graph; there is **no cross-graph or union-of-all-graphs
  build**. On a quad dataset (N-Quads / TriG via `Graph::load_dataset`) each named graph
  is a self-contained `Graph`; fetch one by name with `graph.named_graph(&name)` and
  introspect it alone — `let card = Introspection::build(graph.named_graph(&g).expect("named graph"));` (`named_graph` returns `Option<&Graph>`). Run it
  per graph (or over the default graph) rather than expecting it to merge the quads.

## See also

- `hdt-format` — load `.hdt` archives into a `Graph` before introspecting.
- `fused-decompress-parse`, `rust-parallel-parsing` — fast ingest into the `Graph`
  these crates read (`Graph::load_reader_parallel`).
- `mpc-protocols`, `noir-circuit-patterns`, `verifiable-credentials-zk`,
  `sparql-formal-semantics` — the verifiable/private query estate, orthogonal to this
  retrieval surface.
