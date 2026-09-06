/* tslint:disable */
/* eslint-disable */

/**
 * A forward-only cursor over the N-Triples lines of a CONSTRUCT/DESCRIBE result graph
 * (see [`Store::query_quads_chunks`]): each [`next`](Self::next) yields the next batch of
 * up to `batch_size` triples as an N-Triples fragment (which is also valid Turtle —
 * N-Triples ⊂ Turtle). Concatenating every batch reproduces [`Store::query_quads`]'s full
 * document. The graph is materialised once inside wasm, but each batch string is built on
 * demand and not retained, so the JS-side copy is bounded to one batch at a time.
 */
export class QuadChunks {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * The next N-Triples fragment, or `undefined` when the graph is exhausted.
     */
    next(): string | undefined;
}

/**
 * The ordered chunk sequence of one query result (see [`Store::query_chunks`]):
 * concatenating every chunk yields exactly [`Store::query`]'s JSON string. Chunks
 * split only at solution-row boundaries (~64 KiB flushes), so a consumer can parse
 * rows incrementally without ever holding the whole result as one JS string.
 */
export class QueryChunks {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * The next chunk, or `undefined` when the sequence is exhausted.
     */
    next(): string | undefined;
}

/**
 * A forward-only **cursor over a SELECT result's solution rows** (see
 * [`Store::query_cursor`]): each [`next`](Self::next) yields the next *batch* of up to
 * `batch_size` solutions as a **self-contained** SPARQL 1.1 JSON document — vars in
 * `head`, just that batch's rows in `results.bindings` — so the consumer can `JSON.parse`
 * each batch on its own and process (then drop) it before pulling the next. Unlike
 * [`QueryChunks`], whose chunks are arbitrary byte-cuts of one big JSON string that must
 * be re-joined before parsing, every cursor batch is independently valid. The result is
 * materialised once inside wasm (the engine has no lazy solution iterator at this layer),
 * but each batch's JSON string is built lazily on demand and never retained, so the heavy
 * JS-side string copy is bounded to one batch at a time — never the whole result at once.
 */
export class SolutionCursor {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * The configured batch size (max solutions per [`next`](Self::next)).
     */
    batchSize(): number;
    /**
     * The next batch as a standalone SPARQL 1.1 JSON results document, or `undefined`
     * once every solution has been yielded. A query with zero solutions yields exactly
     * one batch (the empty-`bindings` document) and is then exhausted, so a caller can
     * distinguish "no rows" (one empty batch) from "fully drained" (`undefined`).
     */
    next(): string | undefined;
    /**
     * The total number of solution rows in the (already materialised) result.
     */
    rowCount(): number;
    /**
     * The projected variable names, in order — the `head.vars` shared by every batch.
     */
    vars(): string[];
}

/**
 * An immutable, dictionary-encoded RDF store queryable with SPARQL.
 */
export class Store {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Incremental quad-level delta, mirroring `Graph::apply_delta`: parses
     * `inserts` and `deletes` as N-Quads (N-Triples for default-graph data) and
     * applies them as ONE batch — deletes first, then inserts, routed per graph
     * (named graphs auto-created on first insert) — through the delta overlay:
     * O(batch), no rebuild. Blank nodes denote concrete nodes BY LABEL, so bnode
     * triples CAN be retracted (impossible via SPARQL `DELETE DATA`).
     */
    applyDelta(inserts: string, deletes: string): void;
    /**
     * Answers an **ASK** query as a plain `boolean`, evaluated through the engine's
     * NATIVE ask path ([`sparq_engine::ask`]): the pattern is evaluated under an
     * implicit `LIMIT 1`, so the scan/join **early-exits at the first solution** and
     * nothing is materialised — no SELECT result is built, no SPARQL-JSON string is
     * serialised, and no boolean is parsed back out on the JS side. This is the
     * right entry point for an existence check on a memory-constrained device: prefer
     * it over routing an ASK through [`query`](Self::query) (which would build and
     * serialise the boolean results document) or, worse, rewriting it to a counted
     * `SELECT *`. A non-ASK query (SELECT / CONSTRUCT / DESCRIBE / UPDATE) is rejected
     * with a clear error — use [`query`](Self::query) / [`queryQuads`](Self::query_quads).
     */
    ask(sparql: string): boolean;
    /**
     * Like [`ask`](Self::ask) but under a cooperative working-set budget: any
     * intermediate or final materialised result exceeding `maxRows` rows aborts the
     * query with a `"query budget exceeded (max-rows)"` error rather than running to
     * completion. Use it to bound the worst-case memory an adversarial / accidentally
     * huge ASK pattern can take in the browser tab. The early-exit still applies, so a
     * pattern that finds a solution quickly never approaches the cap. (The engine's
     * other budget dimension, a wall-clock deadline, is native-only — `std::time::Instant`
     * is unusable on `wasm32` — so only the portable row cap is exposed here.)
     */
    askWithMaxRows(sparql: string, max_rows: number): boolean;
    /**
     * Counts the solutions of a SELECT query *without* materialising them — for a
     * single-pattern scan or a two-pattern join the count is read straight from
     * the index (no result rows built). Ideal for "how many?" UI queries on a
     * memory-constrained device.
     */
    count(sparql: string): number;
    /**
     * [SONNET-4.6] sq-q4apb (#2396): derives one complete `FormDescription`
     * (SHACL-to-form, DASH widget scoring) from serialized workspace snapshots —
     * the hosted-web half of the GUI forms bridge (desktop uses the Tauri
     * `derive_form` command; `forms-bridge.ts` feature-detects this method).
     *
     * `data` / `shapes` are RDF documents in `format` (the syntaxes
     * [`Store::load`] accepts; named graphs are preserved dataset-style).
     * `focus` is an absolute IRI or a `_:`-prefixed blank-node label.
     * `options_json` is the snake_case `{"mode": "edit"|"view", "shape"?: …}`
     * object described in the module docs. Stateless one-shot: the receiver's
     * stored triples are not consulted.
     *
     * Returns the derived `FormDescription` as its serde JSON string, verbatim
     * (`JSON.parse` it on the JS side; the frontend must not reconstruct keys,
     * groups, or widgets). Errors — an unparseable graph, focus, shape, or
     * options document — throw a `JsError`; there is no demo-data fallback.
     *
     * Available only when the crate is built with the OPT-IN `forms` feature —
     * the hosted `/app` + site bundle (js `build:wasm`) enables it; the lean
     * default bundle does not, and there this method is simply absent (which is
     * exactly what `forms-bridge.ts` feature-detects).
     */
    deriveForm(data: string, shapes: string, focus: string, format: string, options_json: string): string;
    /**
     * [OPUS-4.8] sq-ncvq.14: query-plan introspection — `EXPLAIN`.
     *
     * Returns the engine's plan for `sparql` as a human-readable string — the
     * algebra tree plus, per BGP, the chosen join order with cardinality
     * estimates, per-step join strategy and pushed-down filters — **without
     * executing the query** (a planning-only dry run; cheap regardless of the
     * query's run cost). This is the same plan text the Rust API
     * (`sparq_engine::explain`) and the HTTP endpoint (`explain` / `explain=plan`
     * query parameter, or `Accept: text/x-sparq-explain`) return, now exposed to
     * JS consumers so the browser/JS surface has the same plan introspection.
     * Works for every query form (SELECT / ASK / CONSTRUCT / DESCRIBE); use
     * [`explainAnalyze`](Self::explain_analyze) to also run and trace it.
     */
    explain(sparql: string): string;
    /**
     * [OPUS-4.8] sq-ncvq.14: query-plan introspection — `EXPLAIN ANALYZE`.
     *
     * Like [`explain`](Self::explain) but **executes** the query (SELECT / ASK
     * only) and appends a per-operator execution trace — output row count per
     * operator, plus totals — after the plan. The returned string matches the
     * Rust API (`sparq_engine::explain_analyze`) and the HTTP `explain=analyze`
     * response. Wall times read 0 on `wasm32` (no monotonic clock — `Instant` is
     * unusable there); the row counts are exact. A CONSTRUCT / DESCRIBE / UPDATE
     * query is rejected with a clear error — use [`explain`](Self::explain) for
     * the graph-valued forms.
     */
    explainAnalyze(sparql: string): string;
    /**
     * STRUCTURED `EXPLAIN ANALYZE` — executes the query (SELECT / ASK only) and
     * returns the typed plan tree as camelCase JSON with each operator's `actual`
     * output rows, wall `nanos` (0 on `wasm32` — no monotonic clock), and `qError`
     * (= max(est/actual, actual/est)) filled in.
     *
     * A CONSTRUCT / DESCRIBE / UPDATE query is rejected with a clear error — use
     * [`explainPlanJson`](Self::explain_plan_json) for the graph-valued forms.
     */
    explainPlanAnalyzeJson(sparql: string): string;
    /**
     * STRUCTURED planning-only `EXPLAIN` — the typed plan tree as camelCase JSON
     * (`operator` / `estimated` / `actual` / `nanos` / `qError` / `children`).
     *
     * A dry run: nothing executes, so every node's `actual` / `nanos` / `qError` is
     * `null` and only the planner's `estimated` cardinalities are populated. Works
     * for every query form (SELECT / ASK / CONSTRUCT / DESCRIBE); a malformed query
     * is rejected with the parser's error.
     */
    explainPlanJson(sparql: string): string;
    /**
     * A rough estimate of the store's in-memory footprint, in bytes.
     */
    heapBytes(): number;
    /**
     * Parses an RDF document into a store. `format`: `"turtle"` | `"ntriples"` |
     * `"nquads"` | `"trig"` | `"jsonld"` (also `"json-ld"` / `"application/ld+json"`,
     * available only when the crate is built with the OPT-IN `jsonld` feature — the
     * site REPL bundle enables it; the lean default bundle does not).
     * Named graphs (from N-Quads / TriG / JSON-LD `@graph`) are folded into the default
     * graph — use [`loadDataset`](Self::load_dataset) to preserve them.
     */
    static load(text: string, format: string): Store;
    /**
     * [FABLE-5] sq-3ul2n.3: like [`load`](Self::load) but ingests **raw bytes** (a JS
     * `Uint8Array`) instead of a JS string, skipping the UTF-16 string round-trip.
     *
     * Pass the `Uint8Array` you already hold — e.g. `new Uint8Array(await
     * response.arrayBuffer())` or `new Uint8Array(await file.arrayBuffer())`. The bytes
     * are copied ONCE into wasm linear memory (no intermediate UTF-16 JS string), then
     * validated as UTF-8 and parsed through the exact same path as [`load`](Self::load),
     * so the resulting store is identical to `Store.load(new TextDecoder().decode(bytes),
     * format)`. `format` is the same set [`load`](Self::load) accepts (`"turtle"` |
     * `"ntriples"` | `"nquads"` | `"trig"` | `"jsonld"` with the opt-in `jsonld` feature).
     * Named graphs are folded into the default graph (as [`load`](Self::load)).
     *
     * Invalid UTF-8 is rejected **fail-closed** with a `JsError` (a `try { … } catch` on
     * the JS side), the same error surface as a malformed document to [`load`](Self::load)
     * — never a panic, abort, or lossy decode.
     */
    static loadBytes(data: Uint8Array, format: string): Store;
    /**
     * [FABLE-5] sq-3ul2n.3: the byte-ingest counterpart to
     * [`loadWithBase`](Self::load_with_base) — ingests raw bytes (a `Uint8Array`) and
     * resolves the document's RELATIVE IRIs against `base`.
     *
     * Identical semantics to [`loadWithBase`](Self::load_with_base) (a document-level
     * `@base` still overrides the supplied `base`; the line-based formats
     * `"ntriples"` / `"nquads"` allow only absolute IRIs so `base` has no effect on them;
     * an invalid `base` is rejected with a `JsError`), only reading bytes instead of a JS
     * string. Invalid UTF-8 is rejected fail-closed exactly as in
     * [`loadBytes`](Self::load_bytes).
     */
    static loadBytesWithBase(data: Uint8Array, format: string, base: string): Store;
    /**
     * Like [`load`](Self::load) but stores the index BLOCK-COMPRESSED (~4-6 B/triple vs
     * 12 — roughly half the index memory, measured −49% on the 6-perm set / −60% on the
     * 3-perm compact set the browser uses). Query results are identical; scans pay a
     * bounded per-block decode (+10–33% on large materialised queries). The right default
     * when the device's RAM, not its CPU, is the binding constraint — i.e. fitting a
     * bigger graph in the tab.
     */
    static loadCompressed(text: string, format: string): Store;
    /**
     * Like [`load`](Self::load) but preserves NAMED GRAPHS from N-Quads / TriG / a
     * JSON-LD `@graph` (with an outer `@id`) as
     * separate sub-graphs, so `GRAPH <iri> { … }` / `GRAPH ?g { … }` patterns,
     * `FROM` / `FROM NAMED` dataset clauses, and SPARQL Updates with `GRAPH`
     * blocks (including `CLEAR GRAPH` / `DROP GRAPH`) all see the dataset.
     * Formats without named graphs ("turtle" / "ntriples") load as [`load`](Self::load).
     * [`size`](Self::size) / [`heapBytes`](Self::heap_bytes) report the DEFAULT
     * graph only (count the dataset with `GRAPH ?g` queries).
     */
    static loadDataset(text: string, format: string): Store;
    /**
     * [OPUS-4.8] sq-f66jz (#1115): like [`load`](Self::load) but resolves the document's
     * RELATIVE IRIs against `base`.
     *
     * A document fetched from a URL (or a SHACL shapes graph / W3C test manifest addressed
     * by its location) often carries relative IRIs and no `@base` of its own; `base` is the
     * base IRI those resolve against — e.g. `loadWithBase("<a> <p> <o> .", "turtle",
     * "http://example.org/dir/")` interns `<http://example.org/dir/a>` etc. A document-level
     * `@base` directive still overrides the supplied `base` (standard Turtle/TriG scoping).
     * The line-based formats (`"ntriples"` / `"nquads"`) allow only absolute IRIs, so `base`
     * has no effect on them. An invalid `base` (not a syntactically valid IRI) is rejected
     * with a `JsError`. Calls straight through to `sparq_core::Graph::load_str_with_base`,
     * so the resolution is byte-identical to the native loader. Named graphs are folded into
     * the default graph (as [`load`](Self::load)); there is no dataset-preserving base
     * variant at this layer yet.
     */
    static loadWithBase(text: string, format: string, base: string): Store;
    /**
     * [OPUS-4.8] sq-ty78o (#1114): a public **empty, mutable** store — the ergonomic
     * `new Store()` constructor.
     *
     * Until now the only way to obtain a `Store` was a static [`load`](Self::load) /
     * [`loadDataset`](Self::load_dataset) / [`loadCompressed`](Self::load_compressed)
     * factory, so a JS caller who wanted to start from nothing and build the graph up with
     * [`updateInPlace`](Self::update_in_place) / [`applyDelta`](Self::apply_delta) had to
     * reach for `Store.load("", "turtle")`. This exposes the natural `new Store()` spelling,
     * returning an empty graph that is immediately mutable through the engine's delta overlay.
     *
     * **Named graphs work out of the box.** The overlay creates a named graph on the first
     * insert that targets it, so `new Store()` then
     * `updateInPlace("INSERT DATA { GRAPH <g> { … } }")` followed by a `GRAPH ?g { … }`
     * query returns the inserted rows — no dataset-mode flag is required for an *empty*
     * store. (Dataset mode matters only when *loading* an existing document whose named
     * graphs would otherwise be folded into the default graph — use
     * [`loadDataset`](Self::load_dataset) for that.) Equivalent to `Store.load("", "turtle")`,
     * surfaced as a `constructor`.
     */
    constructor();
    /**
     * [OPUS-4.8] sq-quly (#796): parses a **SHACL Compact Syntax (SCS)** document
     * into the equivalent SHACL **shapes graph** and returns it as a **pretty
     * Turtle** string.
     *
     * `text` is an SCS document (the W3C compact syntax — `shape`/`shapeClass`,
     * path expressions, `[min..max]`, `nodeKind`, `@`shape-refs, `param=value`,
     * `!`/`|`, nested `{…}` and `[…]`, directives). `base` (optional) is the base
     * IRI that relative IRIs and the `owl:Ontology` subject resolve against; pass
     * `undefined`/`null` for the SCS no-`BASE` convention
     * (`urn:x-base:default`). A document-level `BASE` directive overrides it.
     *
     * The returned Turtle is byte-for-byte what [`serialize`](Self::serialize)
     * produces for the same graph with `("turtle", pretty=true, indent="  ",
     * abbreviate=true)` — a sorted, blank-line-separated, `@prefix`-headed document
     * (the `sh:` / `rdf:` / `rdfs:` / `xsd:` / `owl:` well-known prefixes are
     * compacted). It re-parses as standard Turtle, and the shapes it carries
     * validate data **identically** to the equivalent hand-written Turtle shapes —
     * it is the same triples [`validate`](Self::validate) consumes. This is the
     * SCS *input* counterpart for the playground's "Compact → shapes" mode.
     *
     * This is a **stateless** one-shot — it does not consult the receiver's stored
     * triples (build a throwaway store with `Store.load("", "turtle")` to call it).
     * Errors only when SCS parsing fails (a `JsError` carrying the parser's message
     * + 1-based line); serialising the parsed graph is infallible. Available only
     * when the crate is built with the OPT-IN `scs` feature (which implies `shacl`
     * + `serialize-rdf`) — the site REPL bundle enables it; the lean default bundle
     * does not.
     */
    parseShaclCompact(text: string, base?: string | null): string;
    /**
     * Runs a SELECT query and returns the results as a SPARQL 1.1 JSON string
     * (`application/sparql-results+json`). Benefits from the engine's streaming
     * optimisations: LIMIT stops the scan early, numeric FILTERs are pushed into
     * the scan, OPTIONAL uses a sort-merge join, and COUNT(*) is computed from the
     * index without materialising — all of which matter even more in the browser,
     * where memory and main-thread time are scarce.
     */
    query(sparql: string): string;
    /**
     * Like [`query`](Self::query) but returns the SPARQL 1.1 JSON document as an
     * ordered sequence of ~64 KiB chunks (split only at solution-row boundaries)
     * instead of one string — so large results cross the wasm boundary piecewise
     * and the caller can surface rows incrementally. The chunk sequence is
     * produced eagerly inside wasm (the engine's chunked serialiser, which never
     * concatenates a whole-result string); the streaming win is on the JS side,
     * which holds at most one chunk at a time.
     */
    queryChunks(sparql: string): QueryChunks;
    /**
     * Runs a SELECT (or ASK) query and returns a [`SolutionCursor`] that yields the
     * solutions in batches of at most `batchSize` rows, each batch a self-contained
     * SPARQL 1.1 JSON document the caller can `JSON.parse` on its own. This is the
     * row-oriented streaming entry point: pull a batch, surface/drop its rows, pull the
     * next — the consumer never holds more than one batch, so peak JS memory is bounded
     * by `batchSize` rather than by the whole result. (`queryChunks` streams the *bytes*
     * of one JSON string at fixed ~64 KiB cuts that must be re-joined before parsing;
     * `queryCursor` streams *parseable solution batches*.) `batchSize` is clamped to at
     * least 1. Caveat: the engine materialises the full result inside wasm before the
     * first batch — there is no lazy engine-level solution iterator at this layer — so the
     * bound is on the JS-side string copy, not on wasm working set.
     */
    queryCursor(sparql: string, batch_size: number): SolutionCursor;
    /**
     * Runs a **CONSTRUCT or DESCRIBE** query and returns the resulting RDF graph
     * serialised as **N-Triples** (one `s p o .` line per triple). N-Triples is a
     * syntactic subset of Turtle, so the returned string is also a valid `text/turtle`
     * document. This is the quad-returning entry point: where [`query`](Self::query)
     * answers SELECT/ASK with a solution table, `queryQuads` answers the graph-valued
     * query forms with their constructed graph. CONSTRUCT instantiates its template once
     * per WHERE solution (template blank nodes are freshened per solution, and triples
     * with unbound or RDF-illegal terms are dropped per SPARQL §16.2); DESCRIBE returns
     * the concise bounded description of each described resource. A SELECT/ASK query is
     * rejected here — use [`query`](Self::query) / [`queryChunks`](Self::query_chunks).
     */
    queryQuads(sparql: string): string;
    /**
     * Like [`queryQuads`](Self::query_quads) but returns a [`QuadChunks`] cursor that
     * yields the constructed graph in batches of at most `batchSize` triples (each an
     * N-Triples fragment), so a large constructed/described graph crosses the wasm
     * boundary piecewise and the caller holds at most one batch at a time. Concatenating
     * every batch reproduces `queryQuads`'s document exactly. `batchSize` is clamped to at
     * least 1. Caveat: as with [`queryQuads`](Self::query_quads) the full graph is
     * materialised inside wasm before the first batch; the bound is on the JS-side copy.
     */
    queryQuadsChunks(sparql: string, batch_size: number): QuadChunks;
    /**
     * [OPUS-4.8] sq-fe1s / sq-ixc3.5: serialises the store's contents to a **Turtle**,
     * **TriG**, or **JSON-LD** document string.
     *
     * `format` (case-insensitive) is one of:
     * * `"turtle"` (aliases `"ttl"`, `"text/turtle"`) — the default graph only.
     * * `"trig"` (alias `"application/trig"`) — the whole dataset: default graph at
     *   top level, named graphs as `GRAPH <g> { … }` blocks.
     * * `"jsonld"` / `"json-ld"` / `"application/ld+json"` — JSON-LD 1.1, **expanded**
     *   form by default; `"jsonld-expanded"` / `"jsonld-flattened"` /
     *   `"jsonld-compacted"` pick the form explicitly (`json-ld-…` accepted too).
     *   JSON-LD always emits the whole dataset.
     *
     * When `pretty` is `true` the output is indented: Turtle/TriG use the
     * blank-line-separated, **sorted** (emission-order-independent) `prettyTurtle`
     * shape; JSON-LD uses the structurally re-indented document. The `indent` arg is
     * the indent unit (`undefined`/`null` ⇒ two spaces). When `pretty` is `false` the
     * compact / minified writer is used and `indent` is ignored.
     *
     * `abbreviate` applies to **Turtle/TriG only**: `true` emits a sorted `@prefix`
     * header and compacts IRIs to `prefix:local`; `false` keeps every IRI in full
     * `<…>` form. It is **ignored for JSON-LD** — IRI abbreviation there is selected by
     * the `jsonld-compacted` form (which carries a prefix `@context`), not this flag.
     *
     * `prefixes` is an OPTIONAL caller-supplied prefix map: a JS array of
     * `[prefix, iri]` pairs (e.g. `[["ex", "http://example.org/"], ["schema",
     * "https://schema.org/"]]`). When omitted (`undefined` / `null`) the engine's
     * well-known defaults (`rdf`, `rdfs`, `xsd`, `owl`, `schema` → `http://schema.org/`,
     * `foaf`, `dc`, `skos`, `sh`) are used — **byte-for-byte the prior behaviour**. When
     * supplied, those prefixes drive Turtle/TriG `@prefix` compaction and the JSON-LD
     * compacted `@context` instead, so a caller can match its OWN prefix policy (the
     * site's `COMMON_PREFIXES` with `https://schema.org/` + `dcterms`/`prov`/`geo`/`void`/
     * `ex`, or a query's declared `PREFIX` lines) and get byte-parity output. A malformed
     * entry (not a two-string array) is rejected with a `JsError`. Only used for
     * compaction (Turtle/TriG with `abbreviate=true`, JSON-LD `jsonld-compacted`).
     *
     * This is the document-export counterpart to [`query_quads`](Self::query_quads),
     * which returns a CONSTRUCT/DESCRIBE *result graph* as flat N-Triples: `serialize`
     * writes the **store itself** in a readable syntax. Errors only if `format` is
     * not one of the recognised values, or `prefixes` is malformed (a `JsError`);
     * serialisation itself is infallible. Available only when the crate is built with the
     * OPT-IN `serialize-rdf` feature — the site REPL bundle enables it; the lean default
     * bundle does not. (JSON-LD *serialise-out* needs no extra feature: the writers
     * live under `serialize-rdf`; the `jsonld` feature is INGEST-only.)
     */
    serialize(format: string, pretty: boolean, indent: string | null | undefined, abbreviate: boolean, prefixes?: Array<any> | null): string;
    /**
     * [OPUS-4.8] sq-oy1f.5: serialises the store as a **full W3C JSON-LD 1.1 Compaction**
     * document against a caller-supplied `@context`.
     *
     * Where [`serialize`](Self::serialize)`("jsonld-compacted", …)` only abbreviates IRIs
     * to `prefix:local` CURIEs from a `[prefix, iri]` map (a *prefix-only* `@context`), this
     * applies the real **W3C JSON-LD 1.1 Compaction Algorithm** against the `@context` JSON
     * you pass: **term definitions** (`{"name":"http://…/name"}` or the expanded
     * `{"@id"/"@reverse","@type","@language","@container"}` form), **`@vocab`**, **type
     * coercion** (a term `@type` matching a datatype collapses the value object;
     * `@type":"@id"`/`@vocab` collapse a node reference to a bare IRI string), **language
     * coercion**, **`@container`** (`@set`/`@list`/`@language`/`@index`), **`@reverse`**
     * terms, and `@id`/`@type` keyword aliasing — value + node + IRI compaction against the
     * active context. The whole dataset is emitted (named graphs as nested `@graph` nodes).
     *
     * `context` is the `@context` **JSON text** — e.g. `'{"@vocab":"http://schema.org/"}'`
     * or `'{"name":"http://xmlns.com/foaf/0.1/name"}'`. It must be a JSON **object** (a
     * JSON-LD `@context` value); an empty `{}` yields an expanded-shaped document with no
     * abbreviation. A non-object or malformed JSON is rejected with a `JsError` (never a
     * silently-wrong document).
     *
     * `pretty` selects the indented multi-line shape (whitespace-only re-indentation of the
     * minified document); `indent` is the indent unit (`undefined`/`null` ⇒ two spaces,
     * ignored when `pretty` is `false`).
     *
     * The compaction is **lossless** — every coercion it applies is invertible against the
     * same `@context`, so a JSON-LD-to-RDF round-trip of the output reconstructs the original
     * triples. Routes through the SAME engine writer
     * (`sparq_engine::serialize::graph_to_jsonld_compact`) the native CLI surface uses, so
     * the bytes match. Still **dependency-free** (a hand-rolled `Json` AST — no `serde_json`,
     * no json-ld crate). Available only when the crate is built with the OPT-IN
     * `serialize-rdf` feature (the JSON-LD *serialise-out* path needs no `jsonld` feature —
     * that one is INGEST-only); on the lean default bundle this method is absent.
     */
    serializeCompact(context: string, pretty: boolean, indent?: string | null): string;
    /**
     * Applies a SPARQL 1.1 Update (`INSERT DATA`, `DELETE DATA`, `CLEAR`,
     * `DELETE/INSERT … WHERE` on the default graph) and returns the **new** store —
     * the receiver is immutable and remains valid. Mirrors `sparq_engine::update`'s
     * rebuild semantics. Prefer [`updateInPlace`](Self::update_in_place), which is
     * O(batch) instead of O(store) for the data operations.
     */
    update(sparql: string): Store;
    /**
     * Applies a SPARQL 1.1 Update IN PLACE through the store's delta overlay
     * (`sparq_engine::update_in_place`): data operations are O(batch) per target
     * graph — no index rebuild — and `GRAPH` blocks / graph templates / `CLEAR` /
     * `DROP` / `CREATE` address named graphs. The dictionary grows append-only,
     * so existing term ids stay valid.
     */
    updateInPlace(sparql: string): void;
    /**
     * [OPUS-4.8] sq-yqi1 (#162): validates an RDF **data graph** against a SHACL
     * **shapes graph**, returning a SHACL validation report as a JSON string.
     *
     * Both arguments are RDF documents in the same syntaxes [`Store::load`]
     * accepts (`"turtle"` | `"ntriples"` | `"nquads"` | `"trig"`); they are
     * parsed identically (named graphs folded into the default graph). This is a
     * stateless one-shot — it does not consult the receiver's stored triples —
     * so it is the drop-in replacement for `rdf-validate-shacl`'s
     * `validate(dataDataset, { shapes })`: validation runs through
     * `sparq-shacl`'s SHACL Core + SHACL-SPARQL (`sh:sparql`, §5.2) engine. To
     * validate the triples the store already holds instead, use
     * [`validate_store`](Self::validate_store) (`validateStore`).
     *
     * Returns a JSON object `{ conforms: boolean, results: [...] }`; each result
     * has `focusNode`, `path`, `value`, `sourceShape`,
     * `sourceConstraintComponent`, `severity` and `message` (see the module
     * docs for the exact shape). `JSON.parse` it on the JS side. `sh:conforms`
     * counts EVERY result regardless of severity (the W3C-suite notion); filter
     * `results` by `severity` for a violations-only gate.
     *
     * Errors only if a graph fails to parse (a `JsError` carrying the parse
     * error) — malformed shapes are skipped by the engine, never surfaced as an
     * error. Small-document write-validation (~10–100 triples) sits far below
     * the wasm linear-memory ceiling; very large data graphs should use the
     * server-side HTTP `validate` path instead (#162 path (c)).
     *
     * The `data`/`shapes` arguments take ownership of two parameters; both
     * graphs are dropped when the call returns.
     */
    validate(data: string, shapes: string, format: string): string;
    /**
     * [SONNET-4.6] gh-2520: validates the triples **already loaded in this
     * store** against a SHACL shapes document, returning the same JSON report
     * [`validate`](Self::validate) does.
     *
     * This is the stateful counterpart of [`validate`](Self::validate): the data
     * graph is the receiver's own contents (whatever `load` / `loadDataset` /
     * `update` / `applyDelta` left in it), so a repeat validation — the same
     * store re-checked as shapes are edited — parses only the *shapes* document
     * per call instead of re-parsing the data document every time. `shapes` is an
     * RDF document in any syntax [`Store::load`] accepts (`"turtle"` |
     * `"ntriples"` | `"nquads"` | `"trig"`); the report shape, `sh:conforms`
     * semantics and error behaviour are identical to
     * [`validate`](Self::validate)'s (only a shapes parse failure errors —
     * malformed shapes are skipped by the engine, never surfaced). Given the same
     * two documents the two methods report the same results, *up to blank-node
     * labels*: parsing a shapes document mints fresh labels, so a `sourceShape`
     * naming an anonymous property shape (`_:…`) differs between any two calls —
     * of either method. Treat those labels as per-call identifiers, not stable keys.
     *
     * **Scope:** validation observes the store's **default graph** only. Triples
     * loaded into named graphs by [`load_dataset`](Self::load_dataset) are not
     * focus-node candidates or value nodes here — `load` folds named graphs into
     * the default graph, so a store built that way validates in full. The wasm
     * linear-memory ceiling still applies: validating a very large store is
     * better done server-side (the `sparq-server` HTTP `validate` path).
     */
    validateStore(shapes: string, format: string): string;
    /**
     * The number of (deduplicated) triples in the store.
     */
    readonly size: number;
}

/**
 * Canonicalizes an **N-Quads document** and returns its RDFC-1.0 canonical
 * N-Quads (canonically sorted, one quad per line, blank nodes relabelled to
 * `_:c14nN`, each line `\n`-terminated). Two N-Quads documents that denote
 * RDF-isomorphic datasets — i.e. differ only in blank-node labels and/or quad
 * order — produce byte-identical output, so a caller can hash / compare the
 * result for an isomorphism-aware dataset `equals` / `contains` / content hash.
 *
 * `input` is parsed as N-Quads (the default graph is a 3-term line; named
 * graphs carry their graph term). A malformed document, or one containing an
 * RDF-1.2 triple term (outside the W3C RDFC-1.0 data model), returns the `Err`
 * (`JsError`) arm rather than trapping.
 */
export function canonicalizeNQuads(input: string): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_quadchunks_free: (a: number, b: number) => void;
    readonly __wbg_solutioncursor_free: (a: number, b: number) => void;
    readonly __wbg_store_free: (a: number, b: number) => void;
    readonly canonicalizeNQuads: (a: number, b: number) => [number, number, number, number];
    readonly quadchunks_next: (a: number) => [number, number];
    readonly solutioncursor_batchSize: (a: number) => number;
    readonly solutioncursor_next: (a: number) => [number, number];
    readonly solutioncursor_rowCount: (a: number) => number;
    readonly solutioncursor_vars: (a: number) => [number, number];
    readonly store_applyDelta: (a: number, b: number, c: number, d: number, e: number) => [number, number];
    readonly store_ask: (a: number, b: number, c: number) => [number, number, number];
    readonly store_askWithMaxRows: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly store_count: (a: number, b: number, c: number) => [number, number, number];
    readonly store_deriveForm: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number) => [number, number, number, number];
    readonly store_explain: (a: number, b: number, c: number) => [number, number, number, number];
    readonly store_explainAnalyze: (a: number, b: number, c: number) => [number, number, number, number];
    readonly store_explainPlanAnalyzeJson: (a: number, b: number, c: number) => [number, number, number, number];
    readonly store_explainPlanJson: (a: number, b: number, c: number) => [number, number, number, number];
    readonly store_heapBytes: (a: number) => number;
    readonly store_load: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly store_loadBytes: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly store_loadBytesWithBase: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number];
    readonly store_loadCompressed: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly store_loadDataset: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly store_loadWithBase: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number];
    readonly store_new: () => [number, number, number];
    readonly store_parseShaclCompact: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly store_query: (a: number, b: number, c: number) => [number, number, number, number];
    readonly store_queryChunks: (a: number, b: number, c: number) => [number, number, number];
    readonly store_queryCursor: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly store_queryQuads: (a: number, b: number, c: number) => [number, number, number, number];
    readonly store_queryQuadsChunks: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly store_serialize: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number, number, number];
    readonly store_serializeCompact: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly store_size: (a: number) => number;
    readonly store_update: (a: number, b: number, c: number) => [number, number, number];
    readonly store_updateInPlace: (a: number, b: number, c: number) => [number, number];
    readonly store_validate: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number, number, number];
    readonly store_validateStore: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly querychunks_next: (a: number) => [number, number];
    readonly __wbg_querychunks_free: (a: number, b: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __externref_drop_slice: (a: number, b: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
