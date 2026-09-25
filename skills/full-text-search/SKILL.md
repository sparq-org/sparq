---
name: full-text-search
description: "Full-text search and IRI/label prefix completion via sparq-text: build TextIndex or CompletionIndex and run text: magic predicates inside SPARQL. Use for keyword/prefix/phrase/proximity or opt-in typo-tolerant search, relevance ranking, or entity autocomplete in a sparq Graph."
---

# sparq full-text search (sparq-text)

`sparq-text` is an **opt-in, separate crate** that adds full-text search over the string literals of a sparq `Graph`. It gives you two surfaces: a low-level owned BM25 inverted index (`TextIndex`) keyed by dictionary term id, and `text:` **magic predicates** that you write inside ordinary SPARQL and that `query_text` rewrites into inline `VALUES` over the index's hits. The engine, planner, and the **lean** `sparq-wasm` bundle carry zero text-search code — full-text support exists only when you depend on `sparq-text` (or load its dedicated tier-b `sparq-text-wasm` browser bundle; see below).

[GPT-6] Query entry points and structural rewrites retain VERSION announcements;
see the [version-pinned EBV rules](../sparql-query/ebv-dialects.md). Unsupported
or incompatible labels do not silently select REC 2013.

## Quickstart

`Cargo.toml`:
```toml
[dependencies]
sparq-core = { path = "../sparq-core" }
sparq-text = { path = "../sparq-text" }   # default features: ["engine", "parallel"]
```

```rust
use sparq_core::Graph;
use sparq_text::{query_text, TextIndex};

let g = Graph::load_str(r#"
    <http://ex/post1> <http://ex/title> "The quick brown fox" .
    <http://ex/post2> <http://ex/title> "Fox hunting banned" .
"#, "ntriples").unwrap();

// 1. Library-level: build the BM25 index (one dict scan), search returns Hit { id, score }.
let index = TextIndex::build(&g);
for hit in index.search("quick fox") {          // AND of tokens, BM25 best-first
    let literal = g.dict.term(hit.id);           // hit.id IS the dict id of the literal
    println!("{literal} (bm25 {})", hit.score);
}

// 2. SPARQL-level: text: magic predicates, rewritten + executed by the engine.
let r = query_text(&g, r#"
    PREFIX text: <http://sparq.dev/text#>
    SELECT ?post ?s WHERE {
      ?post  <http://ex/title> ?title .
      ?title text:matches "fox" .
      ?title text:score   ?s .
    } ORDER BY DESC(?s)"#, &index).unwrap();
assert_eq!(r.rows.len(), 2);                     // r.rows: Vec<Vec<Option<oxrdf::Term>>>, r.vars
```

The dictionary term id of a string literal **is** its document id, so search results join back to subjects/predicates through the store's ordinary permutation indexes — that join is what the SPARQL surrounding the magic pattern does.

## Key APIs

Index (always available, `index` module — re-exported at crate root as `TextIndex`, `Hit`, `TermStats`):
- `TextIndex::build(graph: &Graph) -> TextIndex` — cheap 8-B-per-posting index, **no positions** (rayon-sharded under `parallel`; result identical to serial).
- `TextIndex::build_with_positions(graph: &Graph) -> TextIndex` — also records token offsets, enabling `phrase`.
- `TextIndex::with_positions() -> TextIndex` — empty position-enabled index for the delta-fed case (positional counterpart of `TextIndex::default()`).
- **CJK n-gram analyzer (opt-in).** [OPUS-4.8] sq-m3ln The default analyzer splits an unspaced Han run per ideograph, so a multi-char CJK term searches as the (low-precision) AND of its characters. `TextIndex::build_with_analyzer(graph, Analyzer::CjkNgram)` (and `build_with_positions_analyzer` / `with_analyzer` / `with_positions_analyzer` for the positional and delta-fed cases) instead indexes each maximal Han run as its **overlapping character bigrams** (`東京都` → `東京`,`京都`), so `text:matches "東京"` (or `search("東京")`) matches only docs where that pair co-occurs. The index **records its analyzer** (`index.analyzer() -> Analyzer`), so `search`/`phrase`/`apply_delta`/`reconcile`/the `text:` rewrite all tokenize the query the same way. Non-CJK text is byte-for-byte the default analyzer (so a Latin corpus is unchanged); kana/Hangul are not bigrammed (UAX #29 already groups them). Trade-off: a single-ideograph query only matches docs where that char is an *isolated* run — hence opt-in. No new dependencies.
- `TextIndex::search(&self, query: &str) -> Vec<Hit>` — AND of tokens (`*`-suffix = prefix token), BM25-ranked best-first.
- `TextIndex::search_any(&self, query: &str) -> Vec<Hit>` — OR of tokens; scores sum over tokens present.
- `TextIndex::term_stats(&self, token: &str) -> Option<TermStats>` — exact per-token corpus statistics using the index's analyzer: document frequency (`doc_count`), total term frequency (`total_tf`), and the same BM25 inverse document frequency (`idf`) used by search scoring. Returns `None` for absent tokens and input that does not analyze to exactly one token; it does not expand prefixes. [GPT-5.6] sq-rviwn
- With `features = ["fuzzy"]`: `TextIndex::fuzzy(&self, term: &str, max_distance: u8) -> Result<Vec<FuzzyHit>, FuzzyError>` — exactly one analyzed query token; distance must be `0..=2`. A bounded deletion-neighbour index supplies candidates (no full token-dictionary scan), exact Unicode-scalar Levenshtein verifies them, and results order by `(distance asc, BM25 desc, matched term asc, id asc)`. `max_distance = 0` has exactly the `search(term)` hit set and scores. `FuzzyHit { id, distance, score, term }`; defaults/constants are `DEFAULT_MAX_DISTANCE = 1` and `MAX_DISTANCE = 2`. [GPT-5.6] sq-lsp7k.14
- `TextIndex::phrase(&self, query: &str) -> Vec<Id>` — adjacent-and-in-order tokens, ascending ids (unranked). **Panics** if the index has no positions.
- `TextIndex::phrase_near(&self, query: &str, slop: u32) -> Vec<Hit>` — proximity/slop: in-order tokens within a bounded total gap, **relevance-ranked** best-first (`score = 1/(1+gap)`, so adjacency = 1.0). `phrase_near(q, 0)` == `phrase(q)` (each at score 1.0); the hit set grows monotonically with `slop`; order stays significant. Same positions requirement / panic as `phrase`. [OPUS-4.8]
- `TextIndex::apply_delta(&mut self, graph: &Graph, inserts: &[[Term;3]], deletes: &[[Term;3]])` — mirror a `Graph::apply_delta` batch (inserts of new string literals are indexed; deletes are a documented no-op).
- `TextIndex::reconcile(&mut self, graph: &Graph) -> usize` — the **rebuild-on-boot + reconcile** warm fast-path: bring a warm index current with a grown graph in `O(new terms)` by scanning only the dictionary tail appended since the last build/reconcile/delta (the batch-free counterpart of `apply_delta`); returns the number of newly indexed docs, and afterwards `index == TextIndex::build(graph)`. Preserves the positions mode. **Panics** if `needs_rebuild` holds. [OPUS-4.8] sq-oddt
- `TextIndex::indexed_dict_len(&self) -> usize` — generation marker: the graph `dict.len()` the index has indexed up to. `is_consistent_with(&graph) -> bool` — cheap staleness check (has it seen every term the graph now holds?). `needs_rebuild(&graph) -> bool` — flags the one unrepairable case: a **shorter** dictionary (a reopened, durably-recompacted base), mandating a fresh `build`. [OPUS-4.8] sq-oddt
- `TextIndex::has_positions(&self) -> bool`, `len()`, `is_empty()`, `token_count()`, `total_postings()`, `heap_bytes()`. `len()` counts indexed literal documents, `token_count()` counts distinct tokens, and `total_postings()` counts `(token, document)` posting pairs. [GPT-5.6] sq-ukr2k
- `struct Hit { pub id: sparq_core::dict::Id, pub score: f32 }` (`score` comparable within one query only).

Highlighting (default-OFF `highlight` feature — re-exported at crate root as `snippet`, `Snippet`):
- `snippet(text: &str, query_token: &str, context_chars: usize) -> Option<Snippet>` — after resolving a hit id to its literal lexical form, find every occurrence of one default-analyzer token and render a word-trimmed contiguous window with `<mark>` / `</mark>` around each occurrence. Matching is case-insensitive; `Snippet.spans` contains absolute half-open UTF-8 byte ranges into the complete `text`. Empty text, a query that is not exactly one token, and no match return `None`. [GPT-5.6] sq-lsp7k.19

Completion (always available, `complete` module — re-exported at crate root):
- `CompletionIndex::build(&graph)` indexes every IRI under its full IRI and local name,
  plus literal values attached through `rdfs:label` or `skos:prefLabel`.
- `CompletionIndex::apply_delta(&mut self, graph, inserts, deletes)` mirrors newly interned IRIs and inserted `rdfs:label`/`skos:prefLabel` triples after `Graph::apply_delta`; deletes are a documented no-op. `reconcile(&graph)` scans the appended dictionary tail plus label triples involving a new subject or object, while `is_consistent_with` / `needs_rebuild` expose the same append-only staleness contract as `TextIndex`. Label triples added solely between pre-existing terms must be forwarded through `apply_delta`.
- `complete(prefix, k, scores)` returns case-insensitive prefix matches ordered by
  descending injected entity score, then key and id. Pass `None` for lexical ordering.
- Results are `Candidate { id, key, kind, score }`; label candidates carry the subject
  entity id, and `CandidateKind::Label(predicate_id)` records their predicate.
- Matching is prefix-only; fuzzy completion is intentionally outside this index.

SPARQL rewrite (`engine` feature, default on — `rewrite` module):
- `query_text(graph: &Graph, sparql: &str, index: &TextIndex) -> Result<QueryResult, String>` — parse, rewrite `text:` patterns, evaluate.
- `query_text_with_budget(graph, sparql, index, budget: &sparq_engine::QueryBudget) -> Result<QueryResult, String>`.
- `prepare_text(graph, sparql, index) -> Result<sparq_engine::PreparedQuery, String>` — rewrite only; compose with `sparq_engine::ask_prepared` / `construct_prepared` / `query_prepared`.
- `rewrite_query(query: spargebra::Query, graph, index) -> Result<spargebra::Query, String>` — algebra-level rewrite; queries without `text:` patterns pass through unchanged.

Magic predicates (`sparq_text::vocab`, namespace `http://sparq.dev/text#`):
- `?lit text:matches "q"` — `?lit` ranges over literals containing **every** token of `q` (token ending in `*` = prefix).
- `?lit text:matchesAny "q"` — literals containing **at least one** token.
- With `fuzzy`: `?lit text:fuzzy "term"` — typo-tolerant single-token match, default edit distance one; add `?lit text:maxDistance N` on the same subject for `N` in `0..=2`. It is BM25-scored and accepts `text:score ?s`. [GPT-5.6]
- `?lit text:phrase "foo bar"` — literals where the tokens are **adjacent and in order** (needs a positions-enabled index).
- `?lit text:near "foo bar"` — proximity/slop generalisation of `text:phrase`: tokens **in order within a bounded gap**, **relevance-ranked**. Needs positions. Gap budget defaults to 0 (== `text:phrase`); set it with a `text:slop N` companion. Takes an optional `text:score ?s` (the proximity score). [OPUS-4.8]
- `?lit text:slop N` — sets the proximity gap budget for the `text:near` on the same subject variable in the same BGP (a non-negative integer; at most one; valid only beside a `text:near`). [OPUS-4.8]
- `?lit text:score ?s` — binds the relevance score as `xsd:double`: the BM25 score for a `text:matches`/`text:matchesAny`, or the proximity score (`1/(1+gap)`) for a `text:near`. Must accompany exactly one such scored match on the same subject variable in the same BGP. **Not** valid with `text:phrase` (boolean adjacency, unscored). [OPUS-4.8]

## Common recipes

**Prefix / autocomplete search.** A trailing `*` on a query token matches as a prefix (scored as one pseudo-term):
```rust
let hits = index.search("auto*");                       // matches "autonomous", "automatic", ...
// In SPARQL: ?title text:matchesAny "fox auto*"
```

**Typo-tolerant search (default-OFF `fuzzy` feature).** Enable the feature explicitly; ordinary builds contain no fuzzy index or query code:
```toml
sparq-text = { path = "../sparq-text", features = ["fuzzy"] }
```

```rust
let hits = index.fuzzy("quixkly", 1)?; // matches the indexed token "quickly"
// In SPARQL:
//   ?title text:fuzzy "quixkly" .
//   ?title text:maxDistance 1 .
//   ?title text:score ?s .
```

**Highlight a resolved hit (default-OFF `highlight` feature).** The hit carries only a dictionary id and score, so pass the literal text after resolving the id:
```toml
sparq-text = { path = "../sparq-text", features = ["highlight"] }
```

```rust
use sparq_text::snippet;

let text = "the quick fox jumps";
let excerpt = snippet(text, "fox", 10).expect("matching token");
assert_eq!(excerpt.spans, [(10, 13)]);
assert_eq!(excerpt.rendered, "quick <mark>fox</mark> jumps");
```

**Relevance-ranked SPARQL results.** `VALUES` rows carry no order through joins — always sort on a `text:score` variable:
```rust
let r = query_text(&g, r#"
    PREFIX text: <http://sparq.dev/text#>
    SELECT ?post ?s WHERE {
      ?post <http://ex/title> ?title .
      ?title text:matches "brown fox" .
      ?title text:score ?s .
    } ORDER BY DESC(?s)"#, &index)?;
// row[1] is an xsd:double literal; rows are unordered until ORDER BY.
```

**Phrase (adjacency) search.** Build the position-enabled index first; order is significant (`"foo bar"` != `"bar foo"`):
```rust
let pidx = TextIndex::build_with_positions(&g);
let ids = pidx.phrase("quick brown fox");               // literal ids, ascending
// In SPARQL (requires pidx, not the default build):
//   ?title text:phrase "quick brown"
```

**Proximity / slop search (ranked near-phrases).** `text:near` is the scored, bounded-gap variant of `text:phrase` over the same positional index — tokens still in order, but allowed a total gap up to the slop, ranked tightest-first (`score = 1/(1+gap)`):
```rust
let hits = pidx.phrase_near("quick fox", 2);            // Vec<Hit>, best-first; "quick lazy fox" (gap 1) before "quick the lazy fox" (gap 2)
// In SPARQL — text:slop sets the gap budget, text:score binds the proximity score:
//   ?title text:near  "quick fox" .
//   ?title text:slop  2 .
//   ?title text:score ?s .
// ... ORDER BY DESC(?s)
// phrase_near(q, 0) == phrase(q); a larger slop only ever admits more docs.
```

**Incremental upkeep after updates.** Mirror the same batch you fed to the graph:
```rust
g.apply_delta(&inserts, &deletes)?;                     // inserts/deletes: &[[Term;3]]
index.apply_delta(&g, &inserts, &deletes);              // new string literals indexed; deletes no-op
```

**Compose with ASK / CONSTRUCT via prepared queries.** Hits are frozen at rewrite time:
```rust
let prepared = prepare_text(&g,
    r#"PREFIX text: <http://sparq.dev/text#>
       ASK { ?p <http://ex/title> ?t . ?t text:matches "milestones" }"#, &index)?;
let yes = sparq_engine::ask_prepared(&g, &prepared)?;
```

**Pure index, no engine (e.g. lighter dep graph).** Disable default features and use `TextIndex` + `search`/`search_any`/`phrase` directly:
```toml
sparq-text = { path = "../sparq-text", default-features = false }   # or default-features = false, features = ["parallel"]
```

## Gotchas / feature flags / prerequisites

- **Feature flags.** `engine` (default on) brings in the `rewrite` module + `sparq-engine`/`spargebra` (with NO engine default features) — needed for `query_text`/`prepare_text`/`rewrite_query` and all `text:` magic predicates. `engine-builtins` (default on) turns the engine's `regex` (SPARQL REGEX/REPLACE) + `digest` (hash builtins) back on, so a `text:` rewrite over a query that ALSO uses those builtins works. `parallel` (default on) rayon-shards `TextIndex::build` and forwards the engine's parallel execution. `fuzzy` is default-OFF and adds only the bounded deletion-neighbour index/API plus the `text:fuzzy`/`text:maxDistance` rewrite arms. `highlight` is default-OFF and adds only literal snippet extraction; it has no index, engine, or dependency edge. Disable defaults for the bare index (tokenizer + `TextIndex` + BM25) with no engine in the graph; take `features = ["engine"]` ONLY for the lean wasm shape (engine present, but no rayon/regex/digest/fuzzy/highlight — what the `sparq-text-wasm` bundle does). [GPT-5.6] sq-lsp7k.14 sq-lsp7k.19
- **Fuzzy constraints.** `TextIndex::fuzzy` and `text:fuzzy` accept exactly one analyzed token (not a phrase or prefix expression); `max_distance`/`text:maxDistance` must be `0..=2`. Each result is admitted iff exact Levenshtein distance is within the bound. When one literal contains several admitted tokens, its best `(distance, -BM25, term)` candidate wins, so each literal appears once. [GPT-5.6]
- **In-browser ("W-text") bundle.** `crates/sparq-text-wasm` ([OPUS-4.8] sq-jbe6) compiles the `engine`-on `sparq-text` (BM25 index + `text:` rewrite + `sparq-engine`) to `wasm32-unknown-unknown` as a SEPARATE, lazy-loaded bundle (NOT in the lean `sparq-wasm` bundle), for the showcase site's `/surface/full-text` page. It exposes a stateless one-shot `TextSearch`: `query(data, format, sparql)` (parse → build a **positions-enabled** index → rewrite + run the `text:` query → SPARQL-1.1-JSON) and `indexStats(data, format)` (a `{docs,tokens,heapBytes,hasPositions}` footprint — the index-build memory is the bundle's risk, so keep the corpus small). `parallel` is off (no wasm threads); the tokenizer's `unicode-segmentation` is pure-Rust and wasm-portable. Build with `wasm-pack build crates/sparq-text-wasm --target web --release`.
- **What gets indexed.** Only plain/`xsd:string` and language-tagged literals from the graph's dictionary. Typed literals (numbers, dates, `geo:wktLiteral`, ...) are skipped.
- **Per-graph indexes.** Dictionary ids are local to each graph. Named graphs have their own dictionaries — build one `TextIndex` per graph you want searchable, and pass the matching index to `query_text`. Hits come from the index you pass (typically the default graph's).
- **`text:phrase` / `text:near` need positions.** Running either against the cheap `TextIndex::build` index is a **hard query error** ("text:phrase/text:near requires a positions-enabled index; build it with TextIndex::build_with_positions"), not a silent empty result. The bare `TextIndex::phrase(...)` / `phrase_near(...)` methods **panic** in the same situation — guard with `has_positions()` if unsure.
- **Proximity scoring is `1/(1+gap)`.** The `text:near` / `phrase_near` score is a closed form of the document's *tightest* in-order gap (`gap = (last − first) − (n − 1)`): 1.0 for an adjacent occurrence, decreasing monotonically as the best occurrence loosens; ties break by ascending id (the `search` ordering). `slop 0` is exact adjacency (== `text:phrase`), and the hit set is monotonic in slop. Order is significant — no slop makes a reversed phrase match. [OPUS-4.8]
- **Rewrite constraints (each a hard error).** Match/phrase/near subject must be a variable; the query string must be a **constant** literal (no per-row bind-time values); `text:score`'s object must be a variable and must bind exactly one scored match (`text:matches`/`text:matchesAny`/`text:near`, **not** `text:phrase`) on its subject in the same BGP; `text:slop`'s object must be a constant non-negative integer, valid only alongside a `text:near` on the same subject (at most one per near); any unknown IRI under `http://sparq.dev/text#` is rejected (typo guard). A query whose tokens match nothing yields zero rows (empty `VALUES`), not an error.
- **Tokenizer semantics.** UAX #29 Unicode word segmentation + full Unicode lowercasing. No stemming, no stopword list, **no diacritic folding** (`café` != `cafe`). Numbers are tokens. Under the default `Analyzer::Unicode`, an unspaced Han run splits **per ideograph** (`東京都` → `東`,`京`,`都`), so a multi-char CJK term matches as the AND of its characters (use `text:phrase` for adjacency) — opt into `Analyzer::CjkNgram` for character-bigram indexing when that low precision matters (see the CJK n-gram bullet above). [OPUS-4.8] sq-m3ln
- **BM25 scoring.** k1 = 1.2, b = 0.75, idf with the +1 floor. `Hit.score` (and the `text:score` `xsd:double`) is only comparable within a single query. A `*`-prefix token scores as one pseudo-term (its expansions' postings unioned). A wide prefix (e.g. a 4-char prefix matching a third of the corpus) is the dominant cost case — real autocomplete prefixes touch far fewer expansions.
- **Differential BM25 oracle (sparq EXTENSION ratchet — NOT conformance).** [OPUS-4.8] sq-ripcg `crates/sparq-text/tests/bm25_oracle.rs` recomputes the exact BM25 score of every `search`/`search_any` hit with a **from-scratch independent reference scorer** (its own inverted view of the tokenized corpus) and asserts the index reproduces it **bit-for-bit** (`f32::to_bits`) AND in best-first order, over a fixed-seed corpus battery; it pins `TEXT_ORACLE_FLOOR` (the measured count of score-exact assertions, may only RISE). It is wired into the central conformance scoreboard (`sparq-conformance` `scoreboard::SUITES`, the `text-search differential oracle` row, family `sparq extension`) and gated by the `text-oracle` CI job. This is HONESTLY an extension self-consistency ratchet, **not a standards-conformance claim** — there is no normative full-text-over-RDF / BM25 recommendation or test suite. The scoreboard tallies it separately from the standards-conformance total, and the dev-only conformance crate takes **no dependency edge** on `sparq-text` — since [SONNET-4.6] `sq-z1xv8` the floor is a single `sparq_conformance_floors::text::BM25_ORACLE_FLOOR` const that the runner and the scoreboard row both import (a zero-dependency leaf crate), so it is neither mirrored nor read textually. This complements the sibling `tests/oracle.rs`, which checks AND/OR/phrase hit SETS but deliberately not BM25 scores.
- **Deletes + compaction.** `apply_delta` deletions are intentionally a no-op: the dictionary keeps terms, so the incremental index stays exactly equal to a rebuild and orphaned literal ids simply join to zero triples. In-process `Graph::compact` folds the fork structure but **keeps every dictionary id** (it copies records `1..=len` into a fresh frozen base — no renumbering), so an index stays valid across it; `index.reconcile(&graph)` is then a no-op and `needs_rebuild` returns `false`.
- **Durability / sharing — rebuild-on-boot + reconcile contract.** [OPUS-4.8] sq-oddt `TextIndex` has **no on-disk format** and is per-process (not shareable); the durable `Graph` (its WAL/blob) is the source of truth. Under a stateless-core invariant: **boot** rebuilds with `TextIndex::build` over the freshly opened graph (the index is a pure function of the dictionary); a **warm** index kept across updates is brought current by `index.reconcile(&graph)` (`O(new terms)`; the batch-free counterpart of `apply_delta`), after which it equals a fresh `build`. `indexed_dict_len()` / `is_consistent_with(&graph)` are the cheap staleness signals; `needs_rebuild(&graph)` flags the only case `reconcile` cannot repair — a **shorter** dictionary, which arises when a stateless core reopens a *durably*-recompacted base whose persisted store dropped orphaned terms and renumbered ids (so the index's dense id→document mapping no longer lines up; `reconcile` panics there — call `build`).
- **Hits frozen at rewrite time.** `prepare_text`/`rewrite_query` snapshot the index's hits into `VALUES`. Re-prepare after the graph (and index) change.

## See also

- `sparq-vectors` — semantic / approximate-nearest-neighbour (HNSW) search, the sibling opt-in crate for embedding similarity rather than lexical match.
- `sparq-geo` — the same opt-in-separate-crate + `apply_delta` index shape, for spatial predicates.
- `fused-decompress-parse` / `rust-parallel-parsing` — fast ingest into the `Graph` you then index.
