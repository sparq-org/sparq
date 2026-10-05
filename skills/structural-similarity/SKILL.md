---
name: structural-similarity
description: "Training-free structural entity similarity over a sparq RDF graph with the opt-in sparq-sim crate: build an entity's (direction, predicate, neighbor) signature straight from the store's permutation indexes, score weighted Jaccard, Dice, or overlap similarity between two entities, and retrieve the top-k most-similar entities (most_similar) via index-driven candidate generation — no embeddings, no model, no training, correct under incremental updates. Use when adding co-citation / shared-context similarity, predicate-profile (role) similarity, entity/relation linking, or the structural half of a hybrid (structural + text-vector) retrieval over a sparq Graph."
license: MIT
metadata:
  version: "0.1.0"
  homepage: https://github.com/sparq-org/sparq
---

# sparq-sim — structural entity similarity

`sparq-sim` is an **opt-in** crate that scores how structurally similar two RDF
entities are, and retrieves the top-`k` most-similar entities to a query — **without
any embedding model, training, or extra state**. An entity's *structural signature* is
the set of `(direction, predicate, neighbor)` pairs it already participates in (read
directly from sparq-core's SPO / OSP–OPS permutation indexes), and similarity is a
predicate-IDF-weighted Jaccard over two signatures. Because the indexes *are* the
feature store, signatures stay correct under incremental graph updates for free.

It consumes only sparq-core's **public read API** (dict lookups, range scans, the
planner's per-predicate stats). Nothing in the workspace depends on it, and the default
engine build does not compile it.

## Quickstart

`crates/sparq-sim/Cargo.toml` (omit `features` for the original one-hop-only build):

```toml
[dependencies]
sparq-core = { path = "../sparq-core" }
sparq-sim  = { path = "../sparq-sim", features = ["multi-hop"] }
oxrdf = "*"   # for oxrdf::{NamedNode, Term}
```

Score similarity and retrieve nearest entities:

```rust
use sparq_core::Graph;
use sparq_sim::{dice_coefficient, overlap_coefficient, Sim};
use oxrdf::{NamedNode, Term};

# fn main() -> Result<(), String> {
let graph: Graph = /* load RDF */ ;
let sim = Sim::new(&graph);                  // defaults: PredicateNeighbor + IDF

let a = Term::from(NamedNode::new("http://example.org/bolt").unwrap());
let b = Term::from(NamedNode::new("http://example.org/blake").unwrap());

let score = sim.similarity(&a, &b);          // weighted Jaccard in [0, 1]
let top10 = sim.most_similar(&a, 10);        // Vec<(Term, f64)>, best first, self excluded

// Build a signature once and reuse it (probe with an arbitrary signature):
let sig = sim.signature(&a).unwrap();        // Option<Signature>
let by_sig = sim.similar_by_signature(&sig, 10);
let other_sig = sim.signature(&b).unwrap();
let dice = dice_coefficient(&sig, &other_sig);
let overlap = overlap_coefficient(&sig, &other_sig);
let _ = (score, top10, by_sig, dice, overlap);
# Ok(()) }
```

## Public API

- `Sim::new(&graph)` / `Sim::with_config(&graph, SimConfig)` — cheap to construct
  (resolves excluded predicates, snapshots the triple count; predicate frequencies are
  the store's already-computed planner stats, O(1) per lookup).
- `sim.signature(&Term) -> Option<Signature>` — build a signature; `None` for an
  unknown / signature-less term. `Signature::{len, is_empty, total_weight}`.
- `sim.similarity(&a, &b) -> f64` — weighted Jaccard in `[0, 1]`, symmetric.
- `sim.most_similar(&a, k) -> Vec<(Term, f64)>` — best first, the query term excluded.
- `sim.similar_by_signature(&sig, k) -> Vec<(Term, f64)>` — same, probing with a
  pre-built signature (cache signatures across many queries).
- `weighted_jaccard(&sig_a, &sig_b) -> f64` — free function for callers that cache
  signatures themselves.
- `dice_coefficient(&sig_a, &sig_b) -> f64` — weighted Sorensen-Dice coefficient;
  returns `0` when both signatures have zero total weight.
- `overlap_coefficient(&sig_a, &sig_b) -> f64` — weighted
  Szymkiewicz-Simpson coefficient; returns `0` when either signature has zero total
  weight.
- `sim.sketch_index(SketchConfig) -> SketchIndex` (default-off `sketch` Cargo feature)
  — a prebuilt MinHash/LSH index over every entity, the dense-graph escape hatch for
  when `most_similar`'s per-query range-scan candidate generation hits its scaling
  wall. `SketchIndex::most_similar(&a, k)` generates candidates from LSH buckets
  (per-query cost independent of hub-element frequency), ranks them by
  `SketchIndex::estimate(&a, &b)` (the fraction of matching MinHash components — an
  estimate of the *unweighted* Jaccard), then re-scores the top `max(4k, 64)`
  **exactly**: a returned score always equals `similarity(a, entity)`. Recall is
  probabilistic (an entity at unweighted Jaccard `s` becomes a candidate with
  probability `1 − (1 − s^rows)^bands`; identical-signature twins are always found) —
  tune `SketchConfig { num_hashes (128), bands (32), seed }`. Deterministic builds;
  the index snapshots the graph — rebuild after updates.
- `sim.explain_similarity(&a, &b) -> Vec<SharedElement>` (default-off `explain` Cargo
  feature) — the decoded shared signature elements behind a `similarity` /
  `most_similar` score, for reasoning UX ("WHY are these two similar"). Each
  `SharedElement` carries `direction: Direction` (`Out`/`In`), `predicate: Term`,
  `neighbor: Option<Term>` (`None` in `Predicates` mode), and `weight: f64` =
  `min(w_a, w_b)`, the element's contribution to the weighted-Jaccard NUMERATOR — so
  `Σ weight / (total_a + total_b − Σ weight)` reconstructs the exact score. Ordered
  weight-descending (strongest evidence first), then predicate/neighbor/direction id
  ascending; deterministic. Empty for absent terms or disjoint signatures.

### Configuration (`SimConfig`)

- `mode` — `SignatureMode::PredicateNeighbor` (default): similar = **shares concrete
  context** (same team, same games, same birthplace); the mode `most_similar`'s
  index-driven candidate generation is built around. `SignatureMode::Predicates`:
  similar = **used the same way** (predicate-profile / role similarity — two Sports
  share no concrete neighbor, but their profiles are near-identical).
- `idf` (default `true`) — weigh each element by `1 + ln(|G| / freq(pred))`, so sharing
  a rare predicate counts for more than sharing `rdf:type` with half the graph.
  `false` = plain Jaccard.
- `exclude_predicates: Vec<NamedNode>` — drop predicates from signatures entirely (e.g.
  exclude `rdf:type` when evaluating against type ground truth — the leakage rule).
- `max_pair_frequency` (default `10_000`) — candidate-generation hub cap: signature
  elements matched by more than this many triples are **skipped during `most_similar`
  candidate generation only** (the lowest-IDF, least-informative hubs). Exact scoring is
  unaffected. Set to `usize::MAX` for exact-but-slower generation.
- `profile_fallback` (default `true`) — when neighbor-level candidate generation yields
  fewer than `k` results (entities whose elements all point at degree-1 neighbors),
  fill the remaining slots with `Predicates`-style role matches, ranked below every
  exactly-scored result. `false` restores strict v1 (neighbor evidence only).
- `depth` (default `1`, available with the default-off `multi-hop` Cargo feature) — the
  maximum breadth-first structural-neighborhood depth. First-hop elements retain their
  original weight; hop `h` is attenuated by `0.5^(h - 1)`. A value of `0` is treated as
  `1`. Expansion order is deterministic, and the nearest path supplies an element's
  weight when several paths reach it.
- `tbox_aware` (default `true`, available with the default-off `tbox` Cargo feature) —
  expand `rdf:type` signature elements with the graph's own `rdfs:subClassOf` transitive
  closure, and predicate elements with `rdfs:subPropertyOf` closure. Inferred elements
  contribute at `0.5×` their IDF weight (direct elements always win). The closure is
  computed self-containedly from the graph; no extra dependency is added.
- `lexical_fallback` (default `true`, available with the default-off `lexical` Cargo
  feature) — when structural and profile-fallback candidate generation yield fewer than `k`
  results, fill remaining slots with IRI local-name trigram-Jaccard similarity (a sorted
  dictionary index over `TermParts::Iri { suffix }` built at `Sim::with_config` time).
  Lexical candidates rank below every structurally-scored result.

## How it works (cost)

- **Signature** — one SPO range scan (outgoing) + one OSP/OPS range scan (incoming),
  cost `O(log n + degree(e))` at the default depth. With `multi-hop`, the same scans run
  breadth-first for each reachable frontier through `SimConfig::depth`.
- **`most_similar(a, k)`** — candidate generation *through the indexes, not a full
  scan*: each signature element's co-owners are one contiguous index range (POS for
  outgoing `(p, n)`, SPO for incoming). Candidates accumulate shared-element weight (an
  intersection upper bound); the top `max(4k, 64)` are re-scored **exactly**.

## Hybrid retrieval (structural + text)

Structural similarity knows how entities are *connected*; it knows nothing about what
their labels/descriptions *mean*. Pair it with the opt-in
[`vector-search`](../vector-search/SKILL.md) skill (`sparq-vectors`) for the text side
and fuse the two ranked lists — neither crate depends on the other:

```rust
use sparq_sim::Sim;
use sparq_vectors::{fuse_rrf, RRF_K};

let structural = Sim::new(&graph).most_similar(&query, 50);
let text: Vec<(oxrdf::Term, f64)> = /* sparq_vectors::VectorIndex::nearest_term(...) */ vec![];
let hybrid = fuse_rrf(&[&text, &structural], RRF_K, 10);   // rank-based, no normalization
let _ = hybrid;
```

Over-fetch each signal (k = 50 for a top-10 fusion) so the fusion has overlap to
reward. See the [`vector-search`](../vector-search/SKILL.md) skill for `fuse_scores`
(min-max-normalized alpha blend) and the embedding pipeline.

## Honest scope

- **Opt-in, native-only-by-design library** — no SPARQL-level integration (no
  `SERVICE` / magic-predicate / function binding); you call the Rust API directly over a
  `sparq_core::Graph`.
- The **two AUCs differ by design.** In `PredicateNeighbor` mode most same-class pairs
  (two arbitrary athletes) share no concrete neighbor and tie at 0 — that mode measures
  *shared context*, not class membership. Class separation is the `Predicates` mode's
  job; the ranking task the crate is built for (`most_similar`) is measured by
  precision@10, which is high across classes. Use the mode that matches your question.
- `max_pair_frequency` is an **approximation knob on candidate generation only** — it
  never changes the exact score of a returned candidate, but a very rare entity reachable
  only through a hub element could be missed at the default cap.
- **Graph scoping (sq-quuu).** `Sim::new(&graph)` operates on the store of the `Graph`
  it is handed — the **default graph** for the top-level `&graph`, or a **single named
  graph** when you pass that graph's sub-`Graph`. On a quad dataset (N-Quads / TriG via
  `Graph::load_dataset`) each named graph is a self-contained `Graph`; fetch one by name
  with `graph.named_graph(&name)` and build `Sim::new(graph.named_graph(&g)?)`. Signatures
  never reach **across** graphs and there is no union-of-all-graphs mode — choose the graph
  (or the default graph) explicitly; the quads are not merged for you.

_(status: Verified against `crates/sparq-sim/src/lib.rs` + README and the crate's tests
on 2026-07-18 [SONNET-4.6] for sq-181 v1.1 (added default-off `tbox` feature: T-box-aware
signatures via `rdfs:subClassOf`/`rdfs:subPropertyOf` transitive closure, `TBOX_ATTENUATION=0.5`;
and default-off `lexical` feature: sorted IRI local-name index + trigram-Jaccard tertiary
fallback in `most_similar`; both behind `SimConfig::tbox_aware`/`lexical_fallback`; 8 new tests
green in both feature states); also 2026-07-18 [FABLE-5] for sq-lgw (added the default-off
`sketch` feature: `Sim::sketch_index` + `SketchConfig`/`SketchIndex` — MinHash/LSH dense-graph
candidate generation with exact re-scoring, covered by a randomized differential that every
returned score equals `Sim::similarity` plus twin-recall and decline witnesses); previously
2026-07-16 [FABLE-5] for sq-lsp7k (added the default-off `explain` feature:
`Sim::explain_similarity` + `SharedElement`/`Direction`, with a differential test that the
returned weights reconstruct the exact similarity score and a multi-hop min-weight witness);
previously 2026-07-12 [GPT-5.6] for sq-da2bz.
Workspace v0.1.0, opt-in (GenAI phase 1, `research/genai-design.md`), zero `unsafe`
(`#![forbid(unsafe_code)]`). Measured quality/latency gates (same-class precision@10;
`Predicates`-mode class-separation AUC; `most_similar(k=10)` latency) are enforced by
the `olympics_eval` example — run it for the (non-canonical) numbers rather than baking
them here. The entity/relation-linking hook into `sparq-nlq` is now wired (sq-uw40 / #647):
`crates/sparq-nlq/src/link.rs` depends on this crate (`use sparq_sim::Sim;`) and expands each
linked entity with its `Sim::most_similar` structural siblings during NL→SPARQL grounding,
covered by an integration test. Code carries [OPUS-4.8] review markers pending re-review.)_
