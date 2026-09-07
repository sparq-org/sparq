<!-- [OPUS-4.8] sq-inzv: README brought to template. -->
# sparq-text

<p>
  <a href="https://crates.io/crates/sparq-text"><img src="https://img.shields.io/crates/v/sparq-text.svg" alt="crates.io"></a>
  <a href="https://docs.rs/sparq-text"><img src="https://docs.rs/sparq-text/badge.svg" alt="docs.rs"></a>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

Opt-in **full-text search over literals** for the
[sparq](https://github.com/sparq-org/sparq) RDF engine: a small, owned BM25 inverted index
over a `Graph`'s string literals, prefix completion over IRIs and common RDF labels,
plus `text:` **magic predicates** that run text search inside plain SPARQL.

A **separate crate** by design (the `sparq-geo` shape): no existing sparq crate — and
in particular not the wasm build — depends on it. The index is in-house (tokenizer +
posting lists, ~no dependencies — deliberately **not** a tantivy/lucene port): a string
literal's dictionary term id **is** its document id, so search returns matching literal
ids and joining them to subjects/predicates is the store's ordinary permutation-index
work.

## 🚀 Quickstart

```rust
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use sparq_core::Graph;
use sparq_text::TextIndex;

# let ttl = r#"<http://ex/a> <http://www.w3.org/2000/01/rdf-schema#label> "the quick brown fox" ."#;
let mut graph = Graph::load_str(ttl, "turtle")?;
let mut index = TextIndex::build(&graph); // one dict scan; rayon-sharded with `parallel`

let hits = index.search("quick fox");     // AND of tokens, BM25-ranked best-first
let any  = index.search_any("fox dog");   // OR
let auto = index.search("auto*");         // *-suffix = prefix token (autocomplete)
for hit in &hits {
    let literal = graph.dict.term(hit.id); // hit.id IS the dict id; hit.score is BM25
#   let _ = literal;
}
# let _ = (any, auto);

// Phrase / proximity search is OPT-IN: it needs token positions.
let pidx = TextIndex::build_with_positions(&graph);
let ids  = pidx.phrase("quick brown fox");   // adjacent & in order (ascending ids)
let near = pidx.phrase_near("quick fox", 2); // ranked, bounded-gap; score 1/(1+gap)
# let _ = (ids, near);
# Ok(()) }
```

Inside SPARQL (default-on `engine` feature), `query_text` rewrites each magic predicate
into an inline `VALUES` table of the index's hits at the spargebra-algebra level — the
engine, planner, and wasm bundle stay unaware of text search:

```rust
# #[cfg(feature = "engine")]
# fn demo() -> Result<(), Box<dyn std::error::Error>> {
# use sparq_core::Graph;
use sparq_text::{query_text, TextIndex};
# let graph = Graph::load_str(r#"<http://ex/p> <http://ex/title> "the quick brown fox" ."#, "turtle")?;
# let index = TextIndex::build(&graph);
let r = query_text(&graph, r#"
    PREFIX text: <http://sparq.dev/text#>
    SELECT ?post ?s WHERE {
      ?post  <http://ex/title> ?title .
      ?title text:matches "fox" .
      ?title text:score ?s .
    } ORDER BY DESC(?s)"#, &index)?;
# let _ = r;
# Ok(()) }
# fn main() {}
```

## ✨ Features

- **`text:` magic predicates** — `text:matches` (AND), `text:matchesAny` (OR), `text:phrase` (adjacency), `text:near` (proximity/slop, relevance-ranked) with the
  `text:slop N` and `text:score ?s` companions. The query string must be a **constant**
  literal, the match subject a variable, and an unknown `text:` IRI is a hard error.
- **Opt-in fuzzy search** — the default-OFF `fuzzy` feature adds `TextIndex::fuzzy(term, max_distance)` and `text:fuzzy`, backed by bounded deletion-neighbour candidates and exact Levenshtein verification (default one, hard cap two). [GPT-5.6] sq-lsp7k.14
- **BM25 ranking, exact-token semantics** — UAX #29 word segmentation + Unicode
  lowercasing; **no stemming, no stopword list, no diacritic folding** (`café` ≠ `cafe`)
  — language-neutral by design. Only plain / `xsd:string` / language-tagged literals are
  indexed (typed literals skipped); indexes are per-graph. A **differential BM25 oracle**
  (`tests/bm25_oracle.rs`) pins `search`/`search_any` scores + ranking bit-for-bit
  against an independent from-scratch reference scorer, wired into the central
  scoreboard as a `sparq extension` ratchet — **honestly NOT a standards-conformance
  claim** (no normative full-text-over-RDF / BM25 suite exists).
- **IRI and label completion** — `CompletionIndex::build(&graph)` indexes IRIs, local
  names, `rdfs:label`, and `skos:prefLabel`. `complete(prefix, k, scores)` does
  deterministic case-insensitive matching with caller-injected scores; no fuzzy matching.
- **Index metrics / phrase positions** — `len`, `token_count`, and `total_postings` count documents, distinct tokens, and token/document posting pairs. The cheap default (`TextIndex::build`) stores **no**
  positions (8 B per pair); `build_with_positions` enables `phrase` /
  `phrase_near`. A phrase query against a positionless index is a **hard query error**
  (the bare `phrase()` method panics) — only callers that need it pay for it.
- **Incremental upkeep** — `apply_delta` indexes newly inserted string literals
  incrementally; **deletions are a documented no-op** (the dictionary retains terms, so
  the incremental index stays *exactly* equal to a rebuild — pinned by a differential
  test — and orphaned ids simply join to zero triples). `Graph::compact` keeps ids valid.
- **Rebuild-on-boot + reconcile durability contract** — `TextIndex` has **no on-disk
  format** and is not shared between processes; the durable `Graph` is the source of
  truth. Boot rebuilds with `TextIndex::build`; a warm index is brought current in
  `O(new terms)` by `index.reconcile(&graph)`. `needs_rebuild(&graph)` flags the one
  unrepairable case — a reopened, durably-recompacted base whose ids renumbered.
- **Lean browser bundle** — [`sparq-text-wasm`](../sparq-text-wasm/README.md) ships the
  BM25 index + `text:` rewrite to the browser with `features = ["engine"]` only (no
  rayon/regex/digest).

## 📚 Learn more

- **How-to** — [`skills/full-text-search/SKILL.md`](../../skills/full-text-search/SKILL.md)
  (the full predicate table, tokenizer/scoring semantics, and the durability contract).
- **API reference** — [docs.rs/sparq-text](https://docs.rs/sparq-text).
- **Benchmark** — `cargo run --release -p sparq-text --example bench_text` (no figures
  baked in here; query cost is dominated by hits scored — a short prefix over the
  synthetic Zipf vocabulary is a worst case by construction). Tracked figures on the
  [benchmarks dashboard](https://sparq.jeswr.org/dev/bench).
- **Contribute** — [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
