<!-- sq-4lvq: README brought to template (deferred from sq-inzv). -->
# sparq-introspect

**Ontology / schema introspection** for the sparq RDF engine — an **opt-in** crate
(GenAI phase 2) that mines the *effective schema* a knowledge graph actually uses,
straight from the store's sorted permutation indexes. No models, no network, no extra
state, and **no GROUP BY**: every statistic is a sorted scan over indexes the store
already keeps. The output grounds NL→SPARQL prompts (budgeted text summaries; full JSON
for machines) and doubles as planner-grade statistics (the characteristic-set table is
a SOTA star-join cardinality estimator).

## 🚀 Quickstart

```rust,ignore
use sparq_introspect::{facets, FacetRequest, Introspection, BuildOptions};

let ix = Introspection::build(&graph);              // or build_with(&graph, &BuildOptions{..})

ix.classes              // Vec<ClassProfile>      — by instance count, with per-class usage
ix.predicates           // Vec<PredicateProfile>  — global stats + observed/declared domain/range
ix.characteristic_sets  // CharacteristicSets     — top sets + exact tail aggregates
ix.join_hints           // JoinHints              — cross-class (C, p, D) edges + triple counts
ix.entities             // u64                    — distinct typed subjects (void:entities)
ix.vocabularies         // Vocabularies           — namespaces + well-known recognition

ix.to_json()                                  // pretty JSON — the machine surface for LLMs
ix.to_text_summary(2500)                      // prompt-ready digest, most-important-first, ≤ N chars
ix.to_void("http://ex.org/dataset")           // W3C VoID description, as N-Triples (valid Turtle)
ix.to_void_with_cs("http://ex.org/dataset")   // VoID + characteristic-set source stats (scs: ext)
ix.to_shacl()                                 // characteristic sets → W3C SHACL node shapes
ix.schema_summary_for(&seeds, 2500)           // retrieval-mode: schema scoped to seed IRIs

// Persisted *.introspect sidecar — mine once, summarise forever without rescanning:
ix.save(sparq_introspect::sidecar_path_for("data/g.nt"))?;        // → data/g.nt.introspect
let ix = sparq_introspect::Introspection::load("data/g.nt.introspect")?; // O(output) reload

let f = facets(&graph, &FacetRequest { class: Some("http://e/Person".into()),
    top_k: 10, ..Default::default() }); // grouped type/predicate/value distributions
```

## ✨ Features

- **Characteristic sets** (Neumann & Moerkotte, ICDE 2011) — one SPO scan groups
  subjects by their *exact* predicate set; each set carries its subject count,
  per-predicate triple counts (avg multiplicity), and the `rdf:type` histogram of its
  subjects. Top sets retained, exact tail aggregates.
- **Facet counts** (`facets`) — deterministic type/predicate/value distributions; default-off
  `numeric-facets` adds finite XSD numeric min/max/count plus ten equal-width buckets.
- **Schema summary** — classes with instance counts; per-class predicate usage with
  subject/triple counts, **coverage ratios**, and **per-class sample object labels**
  (`ClassPredicate::samples`, drawn only from *this* class's triples, so a minority class
  shows its OWN representative values rather than the predicate's global minimum); plus
  **observed domain/range** (most-common subject/object classes, inferred from usage)
  alongside any **declared** `rdfs:domain`/`rdfs:range` — real KGs are under-declared and
  mis-typed, so usage wins and both are reported.
- **Persisted `*.introspect` sidecar** (`save` / `load` / `from_json`,
  [`sidecar_path_for`]) — write the mined schema as JSON next to the dataset, then reload it
  `O(output)` instead of re-mining `O(|G| + |dict|)`; the format is exactly `to_json`'s, so a
  sidecar is also a plain JSON document. Every export runs off the loaded struct.
- **Cross-class join hints** — the `(subject_class) --predicate--> (object_class)` edge table
  with per-edge triple counts, mined in the *same* SPO scan as the characteristic sets.
- **Text summary** (`to_text_summary(budget_chars)`) — header totals → prefix glossary →
  classes with coverage + range hints + samples → characteristic-set patterns → predicate
  stats. Lines drop greedily at the budget; a final `…` marks elision.
- **VoID export** (`to_void`) — a [W3C VoID](https://www.w3.org/TR/void/) description as
  N-Triples (parses as Turtle too — no serializer dep), with exact `void:triples`,
  `void:entities`, `void:distinctSubjects`, `void:classes`, `void:properties`, and a
  `void:classPartition`/`void:propertyPartition` each. `void:distinctObjects` is **not**
  emitted — omitted rather than misleading (no global de-duplicated count kept).
- **VoID + characteristic-set source stats** (`to_void_with_cs`, federation A3/Z2) — a strict
  superset of `to_void` plus the mined sets under a documented sparq extension vocab `scs:`
  (`<http://sparq.dev/ns/cs#>`); the served federation-descriptor surface that primes a remote
  CostFed/Odyssey-class source-selector with star/multi-join cardinalities. Served at
  `GET /.well-known/void` behind the opt-in `federation-descriptors` feature.
- **SHACL node-shape export** (`to_shacl`) — each mined characteristic set as a W3C
  [SHACL](https://www.w3.org/TR/shacl/) `sh:NodeShape`: a `sh:targetClass` for every class
  **universal** to the set (so the shape auto-applies only where every instance genuinely has
  the predicates), one `sh:PropertyShape` (`sh:path` + `sh:minCount 1`) per non-type
  predicate, and `sh:maxCount 1` exactly when avg multiplicity is 1. Constraints are mined
  from what the data asserts — a data-grounded effective-schema floor, not an aspirational
  contract. Sets with no universal class yield a reusable but target-less shape.
- **ABSTAT-style type minimalization** (`BuildOptions::minimalize_types`, off by default) — folds each subject's types to the most-specific set via the in-graph `rdfs:subClassOf` closure (no OWL/fetch; cycles tolerated); default full-type output unchanged. See the rustdoc.
- **Vocabulary detection** — namespaces in use (split at the last `#`/`/`) with distinct-term
  counts, recognised against a bundled offline table (rdf/rdfs/owl/xsd/foaf/skos/schema.org/
  dcterms/wd/dbo/…).
- **Retrieval-mode summary** (`schema_summary_for(seeds, budget)`) — a seed-scoped digest for
  KGs whose full schema overflows a prompt; filters the already-mined profiles by IRI (no
  re-scan).
- **Graph scoping** — `Introspection::build(&graph)` introspects the store of whatever
  `Graph` it is handed: the **default graph** for the top-level `&graph`, a **single named
  graph** for that graph's sub-`Graph` (fetch by name with
  [`Graph::named_graph(&name)`][named-graph], sq-quuu). There is **no cross-graph or
  union-of-all-graphs** build: a VoID / schema card is always scoped to exactly one graph, so
  on a multi-graph dataset run it per graph rather than expecting it to merge the quads.

  [named-graph]: https://docs.rs/sparq-core/latest/sparq_core/struct.Graph.html#method.named_graph
- **Cost & zero impact** — `O(|G| + |dict|)` time, output-sized memory plus the subject→types
  map. Separate opt-in crate: no core crate depends on it (the server reaches it only through
  opt-in features), and it is read-only over `sparq-core`'s public scan surface.

The `olympics_introspect` example reports load / build / `to_json` / `to_text_summary` time
over the olympics (1.78M triples) and qlever-synthetic (10M) fixtures (paths via
`SPARQ_OLYMPICS_NT` / `SPARQ_SYNTHETIC_NT`); the design-doc §4 gates (scan stays within load
time; summary fast relative to load; the olympics summary names the dataset's actual classes,
asserted by `tests/olympics.rs`). Tracked figures live on the perf dashboard.

```sh
cargo run -p sparq-introspect --example olympics_introspect --release
# add `-- --json <path>` to also write the measurements as machine-readable JSON
# (STDOUT unchanged; timings are advisory/non-canonical, nothing committed)
```

## 📚 Learn more

- Design: [`research/genai-design.md`](../../research/genai-design.md)
- W3C VoID: <https://www.w3.org/TR/void/>

## License

MIT. sq-4lvq
