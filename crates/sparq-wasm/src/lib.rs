//! sparq-wasm: the sparq parser + triplestore + SPARQL engine compiled to
//! WebAssembly, with a minimal bundle (no threads, no serde — results are
//! serialised by hand to SPARQL 1.1 JSON).
//!
//! ```js
//! import init, { Store } from "./sparq_wasm.js";
//! await init();
//! const store = Store.load(turtleText, "turtle");
//! const json = store.query("SELECT * WHERE { ?s ?p ?o } LIMIT 10");
//! const { results } = JSON.parse(json);
//! ```
//!
//! # Query surface (what crosses the wasm boundary)
//!
//! [OPUS-4.8] sq-0ptd (gh-54) — the read-replica / edge-cache tier audit. Every SPARQL
//! query *form* is exported: SELECT/ASK via [`Store::query`] / [`Store::ask`] (plus the
//! streaming [`Store::query_cursor`] / [`Store::query_chunks`]) and **CONSTRUCT / DESCRIBE**
//! via [`Store::query_quads`] / [`Store::query_quads_chunks`] (the constructed graph as
//! N-Triples). The non-regex **string functions** are retained unchanged in the wasm build
//! — `CONTAINS`, `STRSTARTS`, `LCASE` (and `UCASE`, `STRENDS`, `SUBSTR`, `STRLEN`, `CONCAT`,
//! `STRBEFORE`/`STRAFTER`, …): the `Store` shares one `sparq-engine`/`sparq-core` with native,
//! so for these the FILTER / expression evaluator is byte-for-byte the same. (The
//! `regression_string_functions` native test and the `string_functions_retained`
//! headless-wasm test lock the `CONTAINS` / `STRSTARTS` / `LCASE` trio in.)
//!
//! **`REGEX` / `REPLACE` are the one exception — compiled OUT of the lean default bundle.**
//! They sit behind `sparq-engine`'s default-on `regex` Cargo feature, and this crate depends
//! on `sparq-engine` with `default-features = false` to keep the regex automata out of the
//! browser bundle, so on a default-built `Store` a query that uses `REGEX` / `REPLACE` is
//! **rejected** — the engine returns an `"unsupported SPARQL function"` error, surfaced as the
//! `JsError` Err arm (not a silently-empty result). Build the bundle with `--features regex`
//! (or prefer `CONTAINS` / `STRSTARTS` / `STRENDS`) if you need them in wasm.
//!
//! # Persistence is native-only (no `save` / `open` / `mmap` here)
//!
//! The native `sparq_core::Graph` `save` / `open` / `save_compressed` family and the
//! mmap-backed dictionary/store map-in path are **deliberately not exported** to wasm,
//! because they cannot exist on `wasm32`: they take a `std::path::Path` and rely on a POSIX
//! **filesystem** and **`mmap`** (`memmap2`), neither of which a browser/edge wasm sandbox
//! provides. The whole `mmap` Cargo feature is therefore off in this crate's `sparq-core`
//! dependency, and persistence stays on the native (container) tier. A wasm store is built
//! fresh each session from an in-memory document via [`Store::load`] /
//! [`Store::load_compressed`] (the caller already holds the bytes — IndexedDB, `fetch`,
//! a `File`); to *materialise* a store's contents back out for the host to persist, run a
//! `CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }` (or per-graph) through
//! [`Store::query_quads`] and store the N-Triples. There is no binary snapshot format
//! across the boundary.
//!
//! # Runtime constraints (the edge tier's envelope)
//!
//! - **Single-threaded.** No `rayon` in the bundle (wasm has no threads here), so loads and
//!   queries run on the one (main) thread; there is no parallel scan/parse.
//! - **Append-only delta growth.** [`Store::update_in_place`] / [`Store::apply_delta`] apply
//!   mutations through the engine's delta overlay: the permutation indexes and the dictionary
//!   grow **append-only** (existing term ids stay valid — never renumbered), and deletes are
//!   masked rather than reclaimed. Steady-state writing therefore **monotonically grows** the
//!   footprint until a rebuild — re-`load`, or use the immutable-rebuild [`Store::update`]
//!   (which returns a fresh compacted store), to fold the overlay back down and reclaim
//!   deleted space. The wasm tier is read-replica-shaped (load → query), not a long-lived
//!   mutable primary.
//! - **32-bit linear-memory ceiling.** `wasm32` linear memory is **4 GiB** addressable, and a
//!   real browser tab is happier well under ~2 GiB. That — not CPU — is the binding scale
//!   limit; [`Store::load_compressed`] (block-compressed indexes + compact dictionary) is the
//!   lever for fitting a bigger graph under the ceiling, and append-only growth eats into the
//!   same budget. [`Store::heap_bytes`] reports the current footprint so a host can watch it.
//!
//! See `README.md` (the "Browser memory bound" / persistence sections) for the operational
//! detail; this audit is tracked as bead sq-0ptd / gh-54.
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

use sparq_core::Graph;
use wasm_bindgen::prelude::*;

// [OPUS-4.8] sq-yqi1 (#162): the opt-in SHACL `Store::validate(...)` binding.
// Behind the non-default `shacl` feature so the standard bundle carries zero SHACL
// code; the module adds a `#[wasm_bindgen] impl Store` method to the `Store` below.
#[cfg(feature = "shacl")]
mod shacl;

// [OPUS-4.8] sq-fe1s: the opt-in `Store::serialize(format, …)` RDF-writer binding.
// Behind the non-default `serialize-rdf` feature so the lean bundle carries zero
// serializer code; the module adds a `#[wasm_bindgen] impl Store` method to the
// `Store` below, calling straight through to `sparq-engine`'s pretty Turtle / TriG
// writers (byte-identical to the native serialiser).
#[cfg(feature = "serialize-rdf")]
mod serialize;

// [OPUS-4.8] sq-quly (#796): the opt-in `Store::parseShaclCompact(text, base?)`
// SHACL-Compact-Syntax parse binding. Behind the non-default `scs` feature (which
// implies `shacl` + `serialize-rdf`) so the lean bundle carries zero SCS/serializer
// code; the module adds a `#[wasm_bindgen] impl Store` method that parses SCS into a
// shapes Graph via `sparq-shacl` and emits it through the existing `Store::serialize`
// engine-writer path (no new serialiser).
#[cfg(feature = "scs")]
mod scs;

// [SONNET-4.6] sq-q4apb (#2396): the opt-in hosted-web
// `Store.deriveForm(data, shapes, focus, format, optionsJson)` forms bridge.
// Behind the non-default `forms` feature so the lean bundle carries zero
// forms/serde code; the module adds a `#[wasm_bindgen] impl Store` method that
// calls straight through to `sparq-forms`' SHACL-to-form derivation and returns
// the FormDescription serde JSON verbatim (the same contract as the desktop
// Tauri `derive_form` command — gui/app's forms-bridge.ts picks one host).
#[cfg(feature = "forms")]
mod forms;

// [OPUS-4.8] sq-1dd5t (#1047): the opt-in RDFC-1.0 `canonicalizeNQuads(nquads)` free
// function (the @sparq-org/sparq RDF/JS `Dataset` consumes it for isomorphism-aware
// toCanonical / equals / contains). Behind the non-default `canon` feature so the lean
// bundle carries zero canonicalization code; the module exports a `#[wasm_bindgen]` free
// function (not a `Store` method — canonicalization is over an arbitrary quad set, which
// may be a foreign RDF/JS dataset, not necessarily this store's contents).
#[cfg(feature = "canon")]
mod canon;

// [SONNET-4.6] sq-yz27r (#3251): the opt-in `Store.loadJsonLdWithContexts(text, contexts)`
// binding — JSON-LD ingest for a document whose `@context` is given by URL (a Verifiable
// Credential's `"@context": "https://www.w3.org/2018/credentials/v1"`), which the
// no-callback `load(_, "jsonld")` path rejects. Behind the non-default `jsonld-contexts`
// feature so the lean bundle is unchanged; the module adds a `#[wasm_bindgen] impl Store`
// method that installs an `oxjsonld` LoadDocumentCallback over a caller-supplied context
// map. Fail-closed, and it opens no socket — see the module docs for why the fetch has to
// stay on the JS side.
#[cfg(feature = "jsonld-contexts")]
mod jsonld_context;

// Re-export the free function at the crate root so it is reachable as
// `sparq_wasm::canonicalizeNQuads` from the headless wasm test (tests/web.rs) and any
// rlib consumer; `#[wasm_bindgen]` already registers it in the generated JS surface.
#[cfg(feature = "canon")]
pub use canon::canonicalize_nquads;

/// An immutable, dictionary-encoded RDF store queryable with SPARQL.
#[wasm_bindgen]
pub struct Store {
    graph: Graph,
}

/// The ordered chunk sequence of one query result (see [`Store::query_chunks`]):
/// concatenating every chunk yields exactly [`Store::query`]'s JSON string. Chunks
/// split only at solution-row boundaries (~64 KiB flushes), so a consumer can parse
/// rows incrementally without ever holding the whole result as one JS string.
#[wasm_bindgen]
pub struct QueryChunks {
    chunks: std::vec::IntoIter<String>,
}

#[wasm_bindgen]
impl QueryChunks {
    /// The next chunk, or `undefined` when the sequence is exhausted.
    // clippy: a #[wasm_bindgen]-exported inherent method (JS calls `.next()`); it cannot
    // be `Iterator::next`, and renaming would break the published JS binding contract.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<String> {
        self.chunks.next()
    }
}

/// A forward-only **cursor over a SELECT result's solution rows** (see
/// [`Store::query_cursor`]): each [`next`](Self::next) yields the next *batch* of up to
/// `batch_size` solutions as a **self-contained** SPARQL 1.1 JSON document — vars in
/// `head`, just that batch's rows in `results.bindings` — so the consumer can `JSON.parse`
/// each batch on its own and process (then drop) it before pulling the next. Unlike
/// [`QueryChunks`], whose chunks are arbitrary byte-cuts of one big JSON string that must
/// be re-joined before parsing, every cursor batch is independently valid. The result is
/// materialised once inside wasm (the engine has no lazy solution iterator at this layer),
/// but each batch's JSON string is built lazily on demand and never retained, so the heavy
/// JS-side string copy is bounded to one batch at a time — never the whole result at once.
#[wasm_bindgen]
pub struct SolutionCursor {
    result: sparq_engine::QueryResult,
    pos: usize,
    batch_size: usize,
}

#[wasm_bindgen]
impl SolutionCursor {
    /// The next batch as a standalone SPARQL 1.1 JSON results document, or `undefined`
    /// once every solution has been yielded. A query with zero solutions yields exactly
    /// one batch (the empty-`bindings` document) and is then exhausted, so a caller can
    /// distinguish "no rows" (one empty batch) from "fully drained" (`undefined`).
    // clippy: a #[wasm_bindgen]-exported inherent method (JS calls `.next()`); it cannot
    // be `Iterator::next`, and renaming would break the JS binding contract.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<String> {
        let total = self.result.rows.len();
        // Exhausted: emit one empty batch for an empty result (pos 0, total 0), then stop.
        if self.pos > total || (self.pos == total && total != 0) {
            return None;
        }
        let end = (self.pos + self.batch_size).min(total);
        let json = sparq_engine::json::to_sparql_json_rows(
            &self.result.vars,
            &self.result.rows[self.pos..end],
        );
        // Advance past `end`; for an empty result step from 0 to 1 so the next call stops.
        self.pos = if total == 0 { 1 } else { end };
        Some(json)
    }

    /// The projected variable names, in order — the `head.vars` shared by every batch.
    pub fn vars(&self) -> Vec<String> {
        self.result
            .vars
            .iter()
            .map(|v| v.as_str().to_string())
            .collect()
    }

    /// The total number of solution rows in the (already materialised) result.
    #[wasm_bindgen(js_name = rowCount)]
    pub fn row_count(&self) -> usize {
        self.result.rows.len()
    }

    /// The configured batch size (max solutions per [`next`](Self::next)).
    #[wasm_bindgen(js_name = batchSize)]
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }
}

/// A forward-only cursor over the N-Triples lines of a CONSTRUCT/DESCRIBE result graph
/// (see [`Store::query_quads_chunks`]): each [`next`](Self::next) yields the next batch of
/// up to `batch_size` triples as an N-Triples fragment (which is also valid Turtle —
/// N-Triples ⊂ Turtle). Concatenating every batch reproduces [`Store::query_quads`]'s full
/// document. The graph is materialised once inside wasm, but each batch string is built on
/// demand and not retained, so the JS-side copy is bounded to one batch at a time.
#[wasm_bindgen]
pub struct QuadChunks {
    chunks: std::vec::IntoIter<String>,
}

#[wasm_bindgen]
impl QuadChunks {
    /// The next N-Triples fragment, or `undefined` when the graph is exhausted.
    // clippy: a #[wasm_bindgen]-exported inherent method (JS calls `.next()`); it cannot
    // be `Iterator::next`, and renaming would break the JS binding contract.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<String> {
        self.chunks.next()
    }
}

#[wasm_bindgen]
impl Store {
    /// [OPUS-4.8] sq-ty78o (#1114): a public **empty, mutable** store — the ergonomic
    /// `new Store()` constructor.
    ///
    /// Until now the only way to obtain a `Store` was a static [`load`](Self::load) /
    /// [`loadDataset`](Self::load_dataset) / [`loadCompressed`](Self::load_compressed)
    /// factory, so a JS caller who wanted to start from nothing and build the graph up with
    /// [`updateInPlace`](Self::update_in_place) / [`applyDelta`](Self::apply_delta) had to
    /// reach for `Store.load("", "turtle")`. This exposes the natural `new Store()` spelling,
    /// returning an empty graph that is immediately mutable through the engine's delta overlay.
    ///
    /// **Named graphs work out of the box.** The overlay creates a named graph on the first
    /// insert that targets it, so `new Store()` then
    /// `updateInPlace("INSERT DATA { GRAPH <g> { … } }")` followed by a `GRAPH ?g { … }`
    /// query returns the inserted rows — no dataset-mode flag is required for an *empty*
    /// store. (Dataset mode matters only when *loading* an existing document whose named
    /// graphs would otherwise be folded into the default graph — use
    /// [`loadDataset`](Self::load_dataset) for that.) Equivalent to `Store.load("", "turtle")`,
    /// surfaced as a `constructor`.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<Store, JsError> {
        // An empty Turtle document yields an empty default graph whose delta overlay can
        // create named graphs on the first `GRAPH`-targeted insert (round-trip verified).
        let graph = Graph::load_str("", "turtle").map_err(|e| JsError::new(&e))?;
        Ok(Store { graph })
    }

    /// Parses an RDF document into a store. `format`: `"turtle"` | `"ntriples"` |
    /// `"nquads"` | `"trig"` | `"jsonld"` (also `"json-ld"` / `"application/ld+json"`,
    /// available only when the crate is built with the OPT-IN `jsonld` feature — the
    /// site REPL bundle enables it; the lean default bundle does not).
    /// Named graphs (from N-Quads / TriG / JSON-LD `@graph`) are folded into the default
    /// graph — use [`loadDataset`](Self::load_dataset) to preserve them.
    pub fn load(text: &str, format: &str) -> Result<Store, JsError> {
        let graph = Graph::load_str(text, format).map_err(|e| JsError::new(&e))?;
        Ok(Store { graph })
    }

    /// Like [`load`](Self::load) but preserves NAMED GRAPHS from N-Quads / TriG / a
    /// JSON-LD `@graph` (with an outer `@id`) as
    /// separate sub-graphs, so `GRAPH <iri> { … }` / `GRAPH ?g { … }` patterns,
    /// `FROM` / `FROM NAMED` dataset clauses, and SPARQL Updates with `GRAPH`
    /// blocks (including `CLEAR GRAPH` / `DROP GRAPH`) all see the dataset.
    /// Formats without named graphs ("turtle" / "ntriples") load as [`load`](Self::load).
    /// [`size`](Self::size) / [`heapBytes`](Self::heap_bytes) report the DEFAULT
    /// graph only (count the dataset with `GRAPH ?g` queries).
    #[wasm_bindgen(js_name = loadDataset)]
    pub fn load_dataset(text: &str, format: &str) -> Result<Store, JsError> {
        let graph = Graph::load_dataset(text, format).map_err(|e| JsError::new(&e))?;
        Ok(Store { graph })
    }

    /// Like [`load`](Self::load) but stores the index BLOCK-COMPRESSED (~4-6 B/triple vs
    /// 12 — roughly half the index memory, measured −49% on the 6-perm set / −60% on the
    /// 3-perm compact set the browser uses). Query results are identical; scans pay a
    /// bounded per-block decode (+10–33% on large materialised queries). The right default
    /// when the device's RAM, not its CPU, is the binding constraint — i.e. fitting a
    /// bigger graph in the tab.
    #[wasm_bindgen(js_name = loadCompressed)]
    pub fn load_compressed(text: &str, format: &str) -> Result<Store, JsError> {
        let graph = Graph::load_str_compressed(text, format).map_err(|e| JsError::new(&e))?;
        Ok(Store { graph })
    }

    /// [OPUS-4.8] sq-f66jz (#1115): like [`load`](Self::load) but resolves the document's
    /// RELATIVE IRIs against `base`.
    ///
    /// A document fetched from a URL (or a SHACL shapes graph / W3C test manifest addressed
    /// by its location) often carries relative IRIs and no `@base` of its own; `base` is the
    /// base IRI those resolve against — e.g. `loadWithBase("<a> <p> <o> .", "turtle",
    /// "http://example.org/dir/")` interns `<http://example.org/dir/a>` etc. A document-level
    /// `@base` directive still overrides the supplied `base` (standard Turtle/TriG scoping).
    /// The line-based formats (`"ntriples"` / `"nquads"`) allow only absolute IRIs, so `base`
    /// has no effect on them. An invalid `base` (not a syntactically valid IRI) is rejected
    /// with a `JsError`. Calls straight through to `sparq_core::Graph::load_str_with_base`,
    /// so the resolution is byte-identical to the native loader. Named graphs are folded into
    /// the default graph (as [`load`](Self::load)); there is no dataset-preserving base
    /// variant at this layer yet.
    #[wasm_bindgen(js_name = loadWithBase)]
    pub fn load_with_base(text: &str, format: &str, base: &str) -> Result<Store, JsError> {
        let graph = Graph::load_str_with_base(text, format, base).map_err(|e| JsError::new(&e))?;
        Ok(Store { graph })
    }

    /// The number of (deduplicated) triples in the store.
    #[wasm_bindgen(getter)]
    pub fn size(&self) -> usize {
        self.graph.len()
    }

    /// A rough estimate of the store's in-memory footprint, in bytes.
    #[wasm_bindgen(js_name = heapBytes)]
    pub fn heap_bytes(&self) -> usize {
        self.graph.heap_bytes()
    }

    /// Runs a SELECT query and returns the results as a SPARQL 1.1 JSON string
    /// (`application/sparql-results+json`). Benefits from the engine's streaming
    /// optimisations: LIMIT stops the scan early, numeric FILTERs are pushed into
    /// the scan, OPTIONAL uses a sort-merge join, and COUNT(*) is computed from the
    /// index without materialising — all of which matter even more in the browser,
    /// where memory and main-thread time are scarce.
    pub fn query(&self, sparql: &str) -> Result<String, JsError> {
        // Serialise straight from ids to SPARQL-JSON — no intermediate oxrdf::Term per
        // cell (the allocator-bound cost of returning a large result in the browser).
        sparq_engine::query_json(&self.graph, sparql).map_err(|e| JsError::new(&e))
    }

    /// Like [`query`](Self::query) but returns the SPARQL 1.1 JSON document as an
    /// ordered sequence of ~64 KiB chunks (split only at solution-row boundaries)
    /// instead of one string — so large results cross the wasm boundary piecewise
    /// and the caller can surface rows incrementally. The chunk sequence is
    /// produced eagerly inside wasm (the engine's chunked serialiser, which never
    /// concatenates a whole-result string); the streaming win is on the JS side,
    /// which holds at most one chunk at a time.
    #[wasm_bindgen(js_name = queryChunks)]
    pub fn query_chunks(&self, sparql: &str) -> Result<QueryChunks, JsError> {
        let chunks = sparq_engine::query_json_chunks_with_budget(
            &self.graph,
            sparql,
            &sparq_engine::QueryBudget::unlimited(),
        )
        .map_err(|e| JsError::new(&e))?;
        Ok(QueryChunks {
            chunks: chunks.into_iter(),
        })
    }

    /// Runs a SELECT (or ASK) query and returns a [`SolutionCursor`] that yields the
    /// solutions in batches of at most `batchSize` rows, each batch a self-contained
    /// SPARQL 1.1 JSON document the caller can `JSON.parse` on its own. This is the
    /// row-oriented streaming entry point: pull a batch, surface/drop its rows, pull the
    /// next — the consumer never holds more than one batch, so peak JS memory is bounded
    /// by `batchSize` rather than by the whole result. (`queryChunks` streams the *bytes*
    /// of one JSON string at fixed ~64 KiB cuts that must be re-joined before parsing;
    /// `queryCursor` streams *parseable solution batches*.) `batchSize` is clamped to at
    /// least 1. Caveat: the engine materialises the full result inside wasm before the
    /// first batch — there is no lazy engine-level solution iterator at this layer — so the
    /// bound is on the JS-side string copy, not on wasm working set.
    #[wasm_bindgen(js_name = queryCursor)]
    pub fn query_cursor(&self, sparql: &str, batch_size: usize) -> Result<SolutionCursor, JsError> {
        let result = sparq_engine::query(&self.graph, sparql).map_err(|e| JsError::new(&e))?;
        Ok(SolutionCursor {
            result,
            pos: 0,
            batch_size: batch_size.max(1),
        })
    }

    /// Runs a **CONSTRUCT or DESCRIBE** query and returns the resulting RDF graph
    /// serialised as **N-Triples** (one `s p o .` line per triple). N-Triples is a
    /// syntactic subset of Turtle, so the returned string is also a valid `text/turtle`
    /// document. This is the quad-returning entry point: where [`query`](Self::query)
    /// answers SELECT/ASK with a solution table, `queryQuads` answers the graph-valued
    /// query forms with their constructed graph. CONSTRUCT instantiates its template once
    /// per WHERE solution (template blank nodes are freshened per solution, and triples
    /// with unbound or RDF-illegal terms are dropped per SPARQL §16.2); DESCRIBE returns
    /// the concise bounded description of each described resource. A SELECT/ASK query is
    /// rejected here — use [`query`](Self::query) / [`queryChunks`](Self::query_chunks).
    #[wasm_bindgen(js_name = queryQuads)]
    pub fn query_quads(&self, sparql: &str) -> Result<String, JsError> {
        sparq_engine::construct_ntriples(&self.graph, sparql).map_err(|e| JsError::new(&e))
    }

    /// Like [`queryQuads`](Self::query_quads) but returns a [`QuadChunks`] cursor that
    /// yields the constructed graph in batches of at most `batchSize` triples (each an
    /// N-Triples fragment), so a large constructed/described graph crosses the wasm
    /// boundary piecewise and the caller holds at most one batch at a time. Concatenating
    /// every batch reproduces `queryQuads`'s document exactly. `batchSize` is clamped to at
    /// least 1. Caveat: as with [`queryQuads`](Self::query_quads) the full graph is
    /// materialised inside wasm before the first batch; the bound is on the JS-side copy.
    #[wasm_bindgen(js_name = queryQuadsChunks)]
    pub fn query_quads_chunks(
        &self,
        sparql: &str,
        batch_size: usize,
    ) -> Result<QuadChunks, JsError> {
        let triples = sparq_engine::construct_or_describe(&self.graph, sparql)
            .map_err(|e| JsError::new(&e))?;
        let batch = batch_size.max(1);
        let chunks: Vec<String> = triples
            .chunks(batch)
            .map(sparq_engine::triples_to_ntriples)
            .collect();
        Ok(QuadChunks {
            chunks: chunks.into_iter(),
        })
    }

    /// Counts the solutions of a SELECT query *without* materialising them — for a
    /// single-pattern scan or a two-pattern join the count is read straight from
    /// the index (no result rows built). Ideal for "how many?" UI queries on a
    /// memory-constrained device.
    pub fn count(&self, sparql: &str) -> Result<usize, JsError> {
        sparq_engine::count(&self.graph, sparql).map_err(|e| JsError::new(&e))
    }

    /// Answers an **ASK** query as a plain `boolean`, evaluated through the engine's
    /// NATIVE ask path ([`sparq_engine::ask`]): the pattern is evaluated under an
    /// implicit `LIMIT 1`, so the scan/join **early-exits at the first solution** and
    /// nothing is materialised — no SELECT result is built, no SPARQL-JSON string is
    /// serialised, and no boolean is parsed back out on the JS side. This is the
    /// right entry point for an existence check on a memory-constrained device: prefer
    /// it over routing an ASK through [`query`](Self::query) (which would build and
    /// serialise the boolean results document) or, worse, rewriting it to a counted
    /// `SELECT *`. A non-ASK query (SELECT / CONSTRUCT / DESCRIBE / UPDATE) is rejected
    /// with a clear error — use [`query`](Self::query) / [`queryQuads`](Self::query_quads).
    pub fn ask(&self, sparql: &str) -> Result<bool, JsError> {
        sparq_engine::ask(&self.graph, sparql).map_err(|e| JsError::new(&e))
    }

    /// Like [`ask`](Self::ask) but under a cooperative working-set budget: any
    /// intermediate or final materialised result exceeding `maxRows` rows aborts the
    /// query with a `"query budget exceeded (max-rows)"` error rather than running to
    /// completion. Use it to bound the worst-case memory an adversarial / accidentally
    /// huge ASK pattern can take in the browser tab. The early-exit still applies, so a
    /// pattern that finds a solution quickly never approaches the cap. (The engine's
    /// other budget dimension, a wall-clock deadline, is native-only — `std::time::Instant`
    /// is unusable on `wasm32` — so only the portable row cap is exposed here.)
    #[wasm_bindgen(js_name = askWithMaxRows)]
    pub fn ask_with_max_rows(&self, sparql: &str, max_rows: usize) -> Result<bool, JsError> {
        // [OPUS-4.8] `QueryBudget`'s fields are cfg-gated: on `wasm32` the struct has
        // ONLY `max_rows` (the wall-clock `deadline` is native-only — `Instant` panics
        // on wasm32), so `..Default::default()` would be a NEEDLESS struct update there
        // (clippy::needless_update under a wasm-target lint). Start from the unlimited
        // budget and set `max_rows` instead: this is target-agnostic — it fills the
        // native `deadline` (None) without naming it, and degenerates to just `max_rows`
        // on wasm32 — so it is clean under clippy on BOTH targets.
        let mut budget = sparq_engine::QueryBudget::unlimited();
        budget.max_rows = Some(max_rows);
        sparq_engine::ask_with_budget(&self.graph, sparql, &budget).map_err(|e| JsError::new(&e))
    }

    /// Applies a SPARQL 1.1 Update (`INSERT DATA`, `DELETE DATA`, `CLEAR`,
    /// `DELETE/INSERT … WHERE` on the default graph) and returns the **new** store —
    /// the receiver is immutable and remains valid. Mirrors `sparq_engine::update`'s
    /// rebuild semantics. Prefer [`updateInPlace`](Self::update_in_place), which is
    /// O(batch) instead of O(store) for the data operations.
    pub fn update(&self, sparql: &str) -> Result<Store, JsError> {
        let graph = sparq_engine::update(&self.graph, sparql).map_err(|e| JsError::new(&e))?;
        Ok(Store { graph })
    }

    /// Applies a SPARQL 1.1 Update IN PLACE through the store's delta overlay
    /// (`sparq_engine::update_in_place`): data operations are O(batch) per target
    /// graph — no index rebuild — and `GRAPH` blocks / graph templates / `CLEAR` /
    /// `DROP` / `CREATE` address named graphs. The dictionary grows append-only,
    /// so existing term ids stay valid.
    #[wasm_bindgen(js_name = updateInPlace)]
    pub fn update_in_place(&mut self, sparql: &str) -> Result<(), JsError> {
        sparq_engine::update_in_place(&mut self.graph, sparql).map_err(|e| JsError::new(&e))
    }

    /// Incremental quad-level delta, mirroring `Graph::apply_delta`: parses
    /// `inserts` and `deletes` as N-Quads (N-Triples for default-graph data) and
    /// applies them as ONE batch — deletes first, then inserts, routed per graph
    /// (named graphs auto-created on first insert) — through the delta overlay:
    /// O(batch), no rebuild. Blank nodes denote concrete nodes BY LABEL, so bnode
    /// triples CAN be retracted (impossible via SPARQL `DELETE DATA`).
    #[wasm_bindgen(js_name = applyDelta)]
    pub fn apply_delta(&mut self, inserts: &str, deletes: &str) -> Result<(), JsError> {
        self.graph
            .apply_delta_nquads(inserts, deletes)
            .map_err(|e| JsError::new(&e))
    }

    /// [OPUS-4.8] sq-ncvq.14: query-plan introspection — `EXPLAIN`.
    ///
    /// Returns the engine's plan for `sparql` as a human-readable string — the
    /// algebra tree plus, per BGP, the chosen join order with cardinality
    /// estimates, per-step join strategy and pushed-down filters — **without
    /// executing the query** (a planning-only dry run; cheap regardless of the
    /// query's run cost). This is the same plan text the Rust API
    /// (`sparq_engine::explain`) and the HTTP endpoint (`explain` / `explain=plan`
    /// query parameter, or `Accept: text/x-sparq-explain`) return, now exposed to
    /// JS consumers so the browser/JS surface has the same plan introspection.
    /// Works for every query form (SELECT / ASK / CONSTRUCT / DESCRIBE); use
    /// [`explainAnalyze`](Self::explain_analyze) to also run and trace it.
    pub fn explain(&self, sparql: &str) -> Result<String, JsError> {
        sparq_engine::explain(&self.graph, sparql).map_err(|e| JsError::new(&e))
    }

    /// [OPUS-4.8] sq-ncvq.14: query-plan introspection — `EXPLAIN ANALYZE`.
    ///
    /// Like [`explain`](Self::explain) but **executes** the query (SELECT / ASK
    /// only) and appends a per-operator execution trace — output row count per
    /// operator, plus totals — after the plan. The returned string matches the
    /// Rust API (`sparq_engine::explain_analyze`) and the HTTP `explain=analyze`
    /// response. Wall times read 0 on `wasm32` (no monotonic clock — `Instant` is
    /// unusable there); the row counts are exact. A CONSTRUCT / DESCRIBE / UPDATE
    /// query is rejected with a clear error — use [`explain`](Self::explain) for
    /// the graph-valued forms.
    #[wasm_bindgen(js_name = explainAnalyze)]
    pub fn explain_analyze(&self, sparql: &str) -> Result<String, JsError> {
        sparq_engine::explain_analyze(&self.graph, sparql).map_err(|e| JsError::new(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = r#"@prefix ex: <http://ex/> .
        ex:alice ex:name "Alice" ; ex:age 30 ; ex:knows ex:bob .
        ex:bob ex:name "Bob"@en ; ex:age 25 ."#;

    #[test]
    fn select_to_sparql_json() {
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let r = sparq_engine::query(
            &g,
            "PREFIX ex: <http://ex/> SELECT ?n ?a WHERE { ?s ex:name ?n . ?s ex:age ?a } ORDER BY ?a",
        )
        .unwrap();
        let json = sparq_engine::json::to_sparql_json(&r);
        // head vars present
        assert!(json.contains("\"vars\":[\"n\",\"a\"]"));
        // typed literal datatype emitted for the integer age
        assert!(json.contains("\"datatype\":\"http://www.w3.org/2001/XMLSchema#integer\""));
        // language tag emitted for "Bob"@en
        assert!(json.contains("\"xml:lang\":\"en\""));
        // a plain string literal omits the xsd:string datatype
        assert!(json.contains("\"value\":\"Alice\"}"));
        // two solutions
        assert_eq!(json.matches("\"a\":{").count(), 2);
    }

    #[test]
    fn escaping_through_json() {
        // A literal containing a quote and backslash must be JSON-escaped.
        let g = Graph::load_str(
            "@prefix ex: <http://ex/> . ex:a ex:p \"q\\\"x\" .",
            "turtle",
        )
        .unwrap();
        let r = sparq_engine::query(&g, "SELECT ?o WHERE { ?s ?p ?o }").unwrap();
        let json = sparq_engine::json::to_sparql_json(&r);
        assert!(json.contains("\"value\":\"q\\\"x\""), "got: {json}");
    }

    #[test]
    fn uri_and_bnode() {
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let r = sparq_engine::query(
            &g,
            "PREFIX ex: <http://ex/> SELECT ?o WHERE { ?s ex:knows ?o }",
        )
        .unwrap();
        let json = sparq_engine::json::to_sparql_json(&r);
        assert!(json.contains("\"type\":\"uri\",\"value\":\"http://ex/bob\""));
    }

    // ---- sq-0f7: SELECT solution cursor (streaming, batch-level) ----

    /// Concatenating a cursor's batches must surface exactly the rows of the one-shot
    /// `query`, and the cursor must honour the batch size and terminate cleanly.
    #[test]
    fn cursor_batches_cover_all_rows() {
        let store = Store::load(DATA, "turtle").unwrap();
        let q = "PREFIX ex: <http://ex/> SELECT ?s ?n WHERE { ?s ex:name ?n } ORDER BY ?n";
        // batch_size 1 over a 2-row result => two non-empty batches, then exhaustion.
        let mut cur = store.query_cursor(q, 1).unwrap();
        assert_eq!(cur.row_count(), 2);
        assert_eq!(cur.batch_size(), 1);
        assert_eq!(cur.vars(), vec!["s".to_string(), "n".to_string()]);

        let b0 = cur.next().expect("first batch");
        let b1 = cur.next().expect("second batch");
        assert!(
            cur.next().is_none(),
            "cursor must be exhausted after the last batch"
        );

        // Each batch is a self-contained SPARQL-JSON doc with the full head vars.
        for b in [&b0, &b1] {
            assert!(
                b.contains("\"vars\":[\"s\",\"n\"]"),
                "batch missing head vars: {b}"
            );
        }
        // One binding row per batch (batch_size 1), and together they carry both names.
        assert_eq!(b0.matches("\"n\":{").count(), 1);
        assert_eq!(b1.matches("\"n\":{").count(), 1);
        assert!(
            (b0.contains("\"Alice\"") && b1.contains("Bob"))
                || (b1.contains("\"Alice\"") && b0.contains("Bob"))
        );
    }

    /// A batch larger than the result yields everything in one batch; a zero batch size
    /// is clamped to 1 (never an infinite/empty-step loop).
    #[test]
    fn cursor_oversized_and_zero_batch() {
        let store = Store::load(DATA, "turtle").unwrap();
        let q = "PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }";

        let mut big = store.query_cursor(q, 1000).unwrap();
        let only = big.next().expect("single batch");
        assert!(big.next().is_none());
        assert_eq!(
            only.matches("\"n\":{").count(),
            2,
            "oversized batch must hold all rows"
        );

        let mut zero = store.query_cursor(q, 0).unwrap();
        assert_eq!(zero.batch_size(), 1, "batch size clamps to >= 1");
        assert!(zero.next().is_some());
    }

    /// A result with no solutions yields exactly one empty batch (so JS can read head
    /// vars), then terminates — distinguishing "no rows" from "fully drained".
    #[test]
    fn cursor_empty_result_yields_one_empty_batch() {
        let store = Store::load(DATA, "turtle").unwrap();
        let mut cur = store
            .query_cursor(
                "PREFIX ex: <http://ex/> SELECT ?x WHERE { ?s ex:nope ?x }",
                8,
            )
            .unwrap();
        assert_eq!(cur.row_count(), 0);
        let only = cur.next().expect("one empty batch even with zero rows");
        assert!(
            only.contains("\"bindings\":[]"),
            "empty result batch must have empty bindings: {only}"
        );
        assert!(
            cur.next().is_none(),
            "exhausted after the single empty batch"
        );
    }

    // ---- sq-hlq: CONSTRUCT / DESCRIBE -> quads ----

    /// CONSTRUCT through `queryQuads` returns the constructed graph as N-Triples
    /// (a valid Turtle subset): absolute IRIs, `.`-terminated lines.
    #[test]
    fn construct_to_quads_ntriples() {
        let store = Store::load(DATA, "turtle").unwrap();
        let nt = store
            .query_quads(
                "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ?s ex:name ?n }",
            )
            .unwrap();
        // Two name triples -> two constructed triples, each a full N-Triples line.
        assert_eq!(
            nt.lines().filter(|l| !l.trim().is_empty()).count(),
            2,
            "got: {nt}"
        );
        assert!(
            nt.contains("<http://ex/label>"),
            "predicate IRI expanded: {nt}"
        );
        assert!(
            nt.contains("<http://ex/alice> <http://ex/label> \"Alice\" ."),
            "got: {nt}"
        );
        // Language tag preserved on the constructed literal.
        assert!(nt.contains("\"Bob\"@en"), "lang tag preserved: {nt}");
    }

    /// DESCRIBE also flows through `queryQuads` (concise bounded description).
    #[test]
    fn describe_to_quads() {
        let store = Store::load(DATA, "turtle").unwrap();
        let nt = store.query_quads("DESCRIBE <http://ex/bob>").unwrap();
        // CBD of ex:bob = its outgoing triples (name + age), nothing inbound.
        assert!(
            nt.contains("<http://ex/bob> <http://ex/name> \"Bob\"@en ."),
            "got: {nt}"
        );
        assert!(nt.contains("<http://ex/bob> <http://ex/age>"), "got: {nt}");
        assert!(
            !nt.contains("<http://ex/alice>"),
            "CBD must not pull in inbound subjects: {nt}"
        );
    }

    /// `queryQuadsChunks` batches the constructed graph; concatenation == `queryQuads`.
    #[test]
    fn construct_quads_chunks_reassemble() {
        let store = Store::load(DATA, "turtle").unwrap();
        let q = "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ?s ex:name ?n }";
        let whole = store.query_quads(q).unwrap();

        let mut chunks = store.query_quads_chunks(q, 1).unwrap();
        let mut reassembled = String::new();
        let mut n_batches = 0;
        while let Some(c) = chunks.next() {
            n_batches += 1;
            reassembled.push_str(&c);
        }
        assert_eq!(n_batches, 2, "batch_size 1 over 2 triples => 2 batches");
        assert_eq!(
            reassembled, whole,
            "chunked N-Triples must reassemble to the whole document"
        );
    }

    /// A SELECT routed to the quad path is rejected (it is not a graph-valued query).
    /// Asserted against the engine function `queryQuads` delegates to, because the
    /// `JsError`-returning wasm wrapper cannot construct its error on a native target
    /// (`JsError::new` is a wasm-bindgen import that panics off-wasm).
    #[test]
    fn query_quads_rejects_select() {
        let g = Graph::load_str(DATA, "turtle").unwrap();
        assert!(sparq_engine::construct_ntriples(&g, "SELECT ?s WHERE { ?s ?p ?o }").is_err());
    }

    // ---- sq-16a: native ASK early-exit through the wasm layer ----
    //
    // `Store::ask` / `Store::askWithMaxRows` are thin `JsError`-mapping wrappers over
    // `sparq_engine::ask` / `ask_with_budget`. The mapping closure cannot run on a native
    // target (`JsError::new` is a wasm-bindgen import that panics off-wasm), so — exactly as
    // `query_quads_rejects_select` does — these tests assert against the engine functions the
    // exports delegate to: the dispatch (which engine fn, which budget) is what this task wires
    // up, and the engine's own tests already cover `eval_ask`'s LIMIT-1 early-exit semantics.

    /// The ASK dispatch returns the engine's native boolean — true when the pattern has a
    /// solution, false when it does not — without going through the SELECT/JSON path.
    #[test]
    fn ask_dispatches_to_native_bool() {
        let g = Graph::load_str(DATA, "turtle").unwrap();
        assert!(sparq_engine::ask(&g, "PREFIX ex: <http://ex/> ASK { ?s ex:knows ?o }").unwrap());
        assert!(!sparq_engine::ask(&g, "PREFIX ex: <http://ex/> ASK { ?s ex:nope ?o }").unwrap());
        // FILTER is evaluated (not just an existence count): true above the threshold, false below.
        assert!(sparq_engine::ask(
            &g,
            "PREFIX ex: <http://ex/> ASK { ?s ex:age ?a FILTER(?a > 28) }"
        )
        .unwrap());
        assert!(!sparq_engine::ask(
            &g,
            "PREFIX ex: <http://ex/> ASK { ?s ex:age ?a FILTER(?a > 99) }"
        )
        .unwrap());
    }

    /// A non-ASK query routed to the ask path is rejected with a clear error (the message the
    /// `JsError` carries), so `Store::ask` can never silently answer a SELECT/CONSTRUCT.
    #[test]
    fn ask_rejects_non_ask() {
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let err = sparq_engine::ask(&g, "SELECT ?s WHERE { ?s ?p ?o }").unwrap_err();
        assert!(
            err.contains("ASK"),
            "rejection must mention ASK, got: {err}"
        );
        assert!(sparq_engine::ask(
            &g,
            "PREFIX ex: <http://ex/> CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }"
        )
        .is_err());
    }

    /// The budgeted dispatch builds a `max_rows` working-set cap on the portable (wasm-safe)
    /// `QueryBudget` field: a generous cap still answers, a zero cap trips the budget. This is
    /// exactly what `Store::askWithMaxRows` passes to `ask_with_budget`.
    #[test]
    fn ask_with_max_rows_budget() {
        let g = Graph::load_str(DATA, "turtle").unwrap();
        // A two-pattern join materialises an intermediate working set, so the row cap is the
        // thing being enforced (a single-pattern / filter-pushdown ASK answers from the scan
        // without ever building a counted result, so it never approaches the cap — exactly the
        // early-exit win). A generous cap answers; a zero cap trips with a budget error.
        let q = "PREFIX ex: <http://ex/> ASK { ?s ex:knows ?o . ?s ex:age ?a FILTER(?a > 28) }";
        // [OPUS-4.8] Mirror `ask_with_max_rows`: start from `unlimited()` and assign
        // `max_rows`, rather than a struct literal with `..Default::default()`. On wasm32
        // the struct has only the `max_rows` field, so a `..` fill is needless and a
        // wasm-target clippy run flags it (clippy::needless_update); this form is clean
        // on both targets.
        let mut generous = sparq_engine::QueryBudget::unlimited();
        generous.max_rows = Some(1024);
        assert!(sparq_engine::ask_with_budget(&g, q, &generous).unwrap());
        let mut starved = sparq_engine::QueryBudget::unlimited();
        starved.max_rows = Some(0);
        let err = sparq_engine::ask_with_budget(&g, q, &starved).unwrap_err();
        assert!(
            err.contains("budget"),
            "starved budget must report a budget error, got: {err}"
        );
    }

    // ---- sq-ncvq.14: EXPLAIN / EXPLAIN ANALYZE through the wasm layer ----
    //
    // `Store::explain` / `Store::explainAnalyze` are thin `JsError`-mapping wrappers over
    // `sparq_engine::explain` / `explain_analyze`. The mapping closure cannot run on a native
    // target (`JsError::new` is a wasm-bindgen import that panics off-wasm), so — exactly as
    // the quad/ask tests above do — these assert against the engine functions the exports
    // delegate to: this task wires up the dispatch, and the engine's own `explain` module
    // tests already cover the plan text / trace content in detail.

    /// EXPLAIN returns the planning-only plan text — the query form header and the
    /// `Plan:` tree — without executing the query.
    #[test]
    fn explain_returns_plan_text() {
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let plan = sparq_engine::explain(
            &g,
            "PREFIX ex: <http://ex/> SELECT ?n ?a WHERE { ?s ex:name ?n . ?s ex:age ?a }",
        )
        .unwrap();
        assert!(
            plan.contains("EXPLAIN (SELECT)"),
            "plan must name the query form: {plan}"
        );
        assert!(
            plan.contains("Plan:"),
            "plan must include the plan tree header: {plan}"
        );
        // A malformed query surfaces an error (which the wrapper maps to a JsError).
        assert!(sparq_engine::explain(&g, "SELECT WHERE {").is_err());
    }

    /// EXPLAIN ANALYZE returns the plan plus an execution trace for SELECT/ASK, and
    /// rejects the graph-valued forms (CONSTRUCT/DESCRIBE) with a clear error.
    #[test]
    fn explain_analyze_returns_trace_and_rejects_construct() {
        let g = Graph::load_str(DATA, "turtle").unwrap();
        let r = sparq_engine::explain_analyze(
            &g,
            "PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n }",
        )
        .unwrap();
        assert!(
            r.contains("EXPLAIN ANALYZE (SELECT)"),
            "must name analyze + form: {r}"
        );
        assert!(
            r.contains("Plan:"),
            "analyze output must include the plan: {r}"
        );
        // CONSTRUCT/DESCRIBE are explain-only — explain_analyze rejects them.
        let err = sparq_engine::explain_analyze(&g, "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }")
            .unwrap_err();
        assert!(
            err.contains("EXPLAIN ANALYZE"),
            "rejection must mention EXPLAIN ANALYZE, got: {err}"
        );
    }

    // ---- sq-dvyi: JSON-LD ingest through the wasm `Store::load` / `loadDataset` ----

    /// `Store::load(jsonldText, "jsonld")` parses a JSON-LD document and the store is
    /// queryable exactly as a Turtle-loaded one. This is the wasm bundle path the
    /// site REPL's JSON-LD upload/URL feature drives (OPT-IN behind the `jsonld` feature).
    #[cfg(feature = "jsonld")]
    #[test]
    fn jsonld_load_and_query() {
        let doc = r#"{
            "@context": { "ex": "http://ex/" },
            "@id": "ex:alice",
            "ex:name": "Alice",
            "ex:knows": { "@id": "ex:bob" }
        }"#;
        let store = Store::load(doc, "jsonld").unwrap();
        assert_eq!(store.size(), 2);
        let json = store
            .query("PREFIX ex: <http://ex/> SELECT ?n WHERE { ex:alice ex:name ?n }")
            .unwrap();
        assert!(json.contains("\"value\":\"Alice\""), "got: {json}");
        // The IRI object is queryable too.
        let json2 = store
            .query("PREFIX ex: <http://ex/> SELECT ?o WHERE { ex:alice ex:knows ?o }")
            .unwrap();
        assert!(
            json2.contains("\"type\":\"uri\",\"value\":\"http://ex/bob\""),
            "got: {json2}"
        );
    }

    /// `Store::loadDataset(jsonld, "jsonld")` preserves a JSON-LD `@graph` named graph, so
    /// a `GRAPH ?g { … }` query over an uploaded JSON-LD dataset returns the right graph.
    /// OPT-IN behind the `jsonld` feature.
    #[cfg(feature = "jsonld")]
    #[test]
    fn jsonld_load_dataset_named_graph_queryable() {
        let doc = r#"{
            "@id": "http://ex/g1",
            "@graph": [ { "@id": "http://ex/s", "http://ex/p": { "@id": "http://ex/o" } } ]
        }"#;
        let store = Store::load_dataset(doc, "jsonld").unwrap();
        let json = store
            .query("SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }")
            .unwrap();
        assert!(
            json.contains("\"value\":\"http://ex/g1\""),
            "named graph must be queryable: {json}"
        );
    }

    #[test]
    fn compressed_matches_raw() {
        // The compressed browser store must return byte-identical JSON to the raw store,
        // and report a smaller footprint.
        let raw = Store::load(DATA, "turtle").unwrap();
        let cmp = Store::load_compressed(DATA, "turtle").unwrap();
        assert_eq!(raw.size(), cmp.size());
        for q in [
            "PREFIX ex: <http://ex/> SELECT ?n ?a WHERE { ?s ex:name ?n . ?s ex:age ?a } ORDER BY ?a",
            "SELECT ?s ?p ?o WHERE { ?s ?p ?o }",
            "PREFIX ex: <http://ex/> SELECT ?o WHERE { ?s ex:knows ?o }",
        ] {
            assert_eq!(raw.query(q).unwrap(), cmp.query(q).unwrap(), "compressed JSON differs for: {q}");
        }
        assert!(cmp.heap_bytes() <= raw.heap_bytes());
    }

    // ---- [OPUS-4.8] sq-ty78o (#1114): the empty `new Store()` constructor ----

    /// `Store::new()` yields an empty, mutable store (the `new Store()` path), and a
    /// `GRAPH`-targeted `updateInPlace` insert then a `GRAPH ?g` query round-trips — the
    /// named-graph gap the issue reported, fixed purely by the missing constructor (the
    /// overlay creates the named graph on first insert, no dataset flag needed for an empty
    /// store). The `Ok` arm of `new`/`update_in_place` runs natively; the real `wasm32`
    /// export is exercised in `tests/web.rs::new_store_named_graph_roundtrip`.
    #[test]
    fn new_store_empty_then_named_graph_roundtrip() {
        let mut store = Store::new().unwrap();
        assert_eq!(store.size(), 0, "a new Store() is empty");
        store
            .update_in_place(
                "INSERT DATA { GRAPH <http://ex/g> { <http://ex/s> <http://ex/p> <http://ex/o> } }",
            )
            .unwrap();
        // The named graph is queryable: GRAPH ?g returns the inserted row with ?g bound.
        let json = store
            .query("SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }")
            .unwrap();
        assert!(
            json.contains("\"value\":\"http://ex/g\""),
            "named graph round-trips through new Store(): {json}"
        );
        // Equivalent to load("", "turtle"): both start empty and accept the same insert.
        let mut viaload = Store::load("", "turtle").unwrap();
        viaload
            .update_in_place("INSERT DATA { <http://ex/a> <http://ex/p> <http://ex/b> }")
            .unwrap();
        assert_eq!(viaload.size(), 1);
    }

    // ---- [OPUS-4.8] sq-f66jz (#1115): base-IRI load (`loadWithBase`) ----

    /// `Store::loadWithBase` resolves relative IRIs against the supplied base — the gap the
    /// issue reported (the core `load_str_with_base` existed but was unexposed). A relative
    /// subject/object becomes an absolute IRI under the base, and a bogus base is rejected.
    /// The `Ok` arm runs natively; the real `wasm32` export and the `Err` arm are covered in
    /// `tests/web.rs::load_with_base_resolves_relative`.
    #[test]
    fn load_with_base_resolves_relative_iris() {
        let store =
            Store::load_with_base("<a> <p> <../up/o> .", "turtle", "http://ex/dir/").unwrap();
        assert_eq!(store.size(), 1);
        let json = store.query("SELECT ?s ?o WHERE { ?s ?p ?o }").unwrap();
        // <a> resolves under the base dir; <../up/o> resolves one level up.
        assert!(
            json.contains("\"value\":\"http://ex/dir/a\""),
            "relative subject resolved against base: {json}"
        );
        assert!(
            json.contains("\"value\":\"http://ex/up/o\""),
            "relative object resolved (..) against base: {json}"
        );
        // A document-level @base overrides the supplied base (standard Turtle scoping).
        let overridden = Store::load_with_base(
            "@base <http://other/> . <a> <p> <o> .",
            "turtle",
            "http://ex/",
        )
        .unwrap();
        let j2 = overridden.query("SELECT ?s WHERE { ?s ?p ?o }").unwrap();
        assert!(
            j2.contains("\"value\":\"http://other/a\""),
            "@base overrides: {j2}"
        );
        // A syntactically invalid base IRI is an error (mapped to a JsError on wasm).
        assert!(
            Graph::load_str_with_base("<a> <p> <o> .", "turtle", "not a iri").is_err(),
            "an invalid base IRI must error"
        );
    }

    // ---- sq-0ptd (gh-54): string-function retention in the wasm build ----
    //
    // The wasm bundle shares one `sparq-engine` evaluator with native — no string builtin is
    // compiled out for `wasm32` — so this asserts the trio the bead calls out (CONTAINS /
    // STRSTARTS / LCASE) is exercisable through the engine path the `Store::query` wrapper
    // delegates to. (`Store::query` itself maps errors via `JsError::new`, a wasm-bindgen
    // import that panics off-wasm, so — as the other native tests do — this drives
    // `sparq_engine::query_json` directly; the headless `string_functions_retained` test
    // proves the same through the real `#[wasm_bindgen] Store::query` export in wasm.)
    #[test]
    fn regression_string_functions() {
        let data = r#"@prefix ex: <http://ex/> .
            ex:a ex:name "Alice" . ex:b ex:name "BOB" . ex:c ex:name "carol" ."#;
        let g = Graph::load_str(data, "turtle").unwrap();

        // STRSTARTS: only "Alice" starts with "Al".
        let j = sparq_engine::query_json(
            &g,
            r#"PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n FILTER(STRSTARTS(?n, "Al")) }"#,
        )
        .unwrap();
        assert!(j.contains("\"value\":\"Alice\""), "STRSTARTS: {j}");
        assert_eq!(j.matches("\"n\":{").count(), 1, "STRSTARTS one row: {j}");

        // CONTAINS over LCASE: lowercasing folds "BOB"->"bob" and "carol", both contain "o".
        let j = sparq_engine::query_json(
            &g,
            r#"PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n FILTER(CONTAINS(LCASE(?n), "o")) }"#,
        )
        .unwrap();
        assert_eq!(
            j.matches("\"n\":{").count(),
            2,
            "CONTAINS(LCASE(..)) matches BOB + carol: {j}"
        );

        // LCASE as a projected value (BIND), not just inside a FILTER.
        let j = sparq_engine::query_json(
            &g,
            r#"PREFIX ex: <http://ex/> SELECT ?l WHERE { ex:b ex:name ?n BIND(LCASE(?n) AS ?l) }"#,
        )
        .unwrap();
        assert!(
            j.contains("\"value\":\"bob\""),
            "LCASE projects lowercase: {j}"
        );
    }

    // [OPUS-4.8] sq-0ptd (gh-54): the NEGATIVE half of the audit — "REGEX/REPLACE are
    // compiled OUT of the lean default bundle" — is asserted ONLY on the real `wasm32`
    // target, in `tests/web.rs::regex_compiled_out_by_default`. It is NOT a native unit
    // test, and deliberately so: `sparq-engine`'s `regex` feature is part of its DEFAULT
    // set, and many OTHER workspace crates (sparq-mpc, sparq-prov, sparq-bench, …) depend
    // on `sparq-engine` with default features. In ANY native `cargo nextest --workspace`
    // build — which is exactly what the CI test-archive lane runs — Cargo FEATURE
    // UNIFICATION compiles a SINGLE `sparq-engine` with `regex` ON, so a native query that
    // uses REGEX SUCCEEDS regardless of this crate's own `regex` feature. A native negative
    // assertion is therefore unobservable/unsound (a `not(feature = "shacl")` guard does not
    // help — shacl is not the only unification source). Only the `wasm32` build links
    // `sparq-wasm`'s own (no-defaults, regex-off) `sparq-engine` without unifying the rest of
    // the workspace, so regex IS genuinely compiled out there and the negative is testable.

    /// With `--features regex` the wasm crate forwards to `sparq-engine/regex` and REGEX works
    /// — the positive half of the audit, asserting the opt-in lever actually re-enables it.
    #[cfg(feature = "regex")]
    #[test]
    fn regex_present_when_enabled() {
        let data = r#"@prefix ex: <http://ex/> . ex:a ex:name "Alice" . ex:b ex:name "Bob" ."#;
        let g = Graph::load_str(data, "turtle").unwrap();
        let j = sparq_engine::query_json(
            &g,
            r#"PREFIX ex: <http://ex/> SELECT ?n WHERE { ?s ex:name ?n FILTER(REGEX(?n, "^Al")) }"#,
        )
        .unwrap();
        assert!(
            j.contains("\"value\":\"Alice\""),
            "REGEX matches Alice: {j}"
        );
        assert_eq!(
            j.matches("\"n\":{").count(),
            1,
            "REGEX keeps only Alice: {j}"
        );
    }
}

// [FABLE-5] sq-586sh (#890 ask A): the opt-in ODRL usage-control PROBE binding —
// `sparq-policy` compiled to wasm32 behind the non-default `policy` feature, so the
// lean bundle carries zero policy code. Two EXPERIMENTAL free functions
// (`policyEvaluate` / `policyConflicts`); the full JS API awaits a maintainer
// public-contract decision. Declared at the END of the file (not beside the other
// feature-gated `mod`s above) so a feature-OFF build's `line!()`/`Location` info for
// everything in this file is unmoved — the lean wasm bundle stays BYTE-identical,
// not merely size-identical (the cfg-gated-token line-drift trap).
#[cfg(feature = "policy")]
mod policy;

// Re-export at the crate root (mirrors `canon`) so the exports are reachable from a
// headless wasm test and any rlib consumer; `#[wasm_bindgen]` already registers them
// in the generated JS surface.
#[cfg(feature = "policy")]
pub use policy::{policy_conflicts, policy_evaluate};

// [FABLE-5] sq-ixc3.19: the opt-in STRUCTURED-explain bindings —
// `Store::explainPlanJson` / `Store::explainPlanAnalyzeJson` return the engine's typed
// plan tree (`sparq_engine::explain_json::PlanNode`, sq-u4lgr/#902) as camelCase JSON
// for the GUI plan explorer, behind the non-default `explain-json` feature (zero new
// dependencies). Declared at the END of the file (the `policy` pattern above) so a
// feature-OFF build's `line!()`/`Location` info for everything in this file is
// unmoved — the lean wasm bundle stays BYTE-identical, not merely size-identical
// (the cfg-gated-token line-drift trap).
#[cfg(feature = "explain-json")]
mod explain_plan;

// [FABLE-5] sq-3ul2n.3: the opt-in `Store::loadBytes` / `loadBytesWithBase` byte-ingest
// bindings — a `Uint8Array` (from `response.arrayBuffer()` / `File.arrayBuffer()`) is
// copied ONCE into linear memory (no UTF-16 JS-string round-trip), validated as UTF-8
// fail-closed, and fed the SAME parse path as `load`. Behind the non-default
// `bytes-ingest` feature (zero new dependencies) so the lean bundle carries zero
// byte-ingest code. Declared at the END of the file (the `policy` / `explain_plan`
// pattern above) so a feature-OFF build's `line!()`/`Location` info for everything in
// this file is unmoved — the lean wasm bundle stays BYTE-identical, not merely
// size-identical (the cfg-gated-token line-drift trap).
#[cfg(feature = "bytes-ingest")]
mod bytes;
