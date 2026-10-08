// [OPUS-4.8] sq-2e93 — the site's wasm-engine entry point.
//
// The framework-agnostic core — the ONE `WasmStore` type declaration, the wasm loaders
// (`loadSparq`/`prewarmSparq`), the SPARQL-JSON + SHACL shapes, the `match()`/cursor query
// helpers, `sparqShaclValidate` and `formatTerm` — now lives in the shared
// `@sparq/client` package (`packages/sparq-client`). This removes the hand-redeclared
// `WasmStore` drift that `research/gui-design.md` §0/§4 flagged: there is now a single
// source for that surface, which the site re-exports here so every existing
// `@/lib/sparq-wasm` import keeps resolving unchanged, and which the (proposed) Tauri 2 GUI
// consumes directly rather than becoming a third hand-copy.
//
// What stays HERE is only the site-specific glue that is coupled to the site's
// dataset-format knowledge (`./repl-dataset`): the named-graph-preserving load entry point
// and the all-quads (de)serialisation used by the "add to current" merge. Those take an
// opinion on which RDF formats carry named graphs, which is a site concern, not part of the
// engine's framework-agnostic surface.

import {
  loadSparq,
  termValue,
  type SparqlResults,
  type WasmModule,
  type WasmStore,
} from "@sparq/client";

import {
  ALL_QUADS_BODY,
  ALL_QUADS_QUERY,
  isDatasetFormat,
  rowsToNQuads,
} from "./repl-dataset";

// Re-export the shared framework-agnostic client surface so existing `@/lib/sparq-wasm`
// importers across the site keep working with no call-site change.
export {
  type CursorBatch,
  type LoadSparqOptions,
  type MatchTerm,
  type ResultTable,
  type ShaclReport,
  type ShaclResult,
  type SparqlBinding,
  type SparqlResults,
  type SparqlTerm,
  type SparqlTripleTerm,
  type WasmModule,
  type WasmSolutionCursor,
  type WasmStore,
  type WasmStoreCtor,
  countQuads,
  formatTerm,
  isTripleTerm,
  termToNTriples,
  termValue,
  loadSparq,
  matchQuads,
  prewarmSparq,
  // [OPUS-4.8] sq-4296 (#935 / #981) — the IDLE-deferred pre-warm: schedule the wasm fetch on
  // the next browser-idle slot so the ~300 kB+ engine never blocks the initial page paint.
  prewarmSparqWhenIdle,
  type IdlePrewarmHandle,
  sparqShaclValidate,
  // [OPUS-4.8] sq-pyn7 (#796) — the SCS *input* helper: parse a SHACL Compact Syntax document
  // into a Turtle shapes graph via the `scs`-enabled wasm `Store.parseShaclCompact`, the
  // counterpart of the `shapesToCompact` Turtle→SCS serialiser.
  sparqParseShaclCompact,
  streamQueryRows,
  // [OPUS-4.8] sq-x0kp — the framework-agnostic results-formatting core (table extraction,
  // CSV/TSV export, raw-JSON pretty-print, ASK discrimination) the REPL result panel draws.
  extractTable,
  resultVars,
  resultRows,
  isAskResult,
  askValue,
  resultsToCsv,
  resultsToTsv,
  formatSparqlJson,
} from "@sparq/client";

/**
 * [OPUS-4.8] sq-17nw — parses RDF into a store, PRESERVING named graphs for the
 * quad-bearing formats (N-Quads / TriG / JSON-LD) by routing them through `loadDataset`
 * instead of `load` (which folds every named graph into the default graph). Triple-only
 * formats (`turtle` / `ntriples`) carry no named graphs, so the cheaper `load` is used.
 * This is the single load entry point the REPL uses for every source (default, picker,
 * upload, URL, and the re-load step of a merge). The dataset-format decision lives in the
 * site (`./repl-dataset`), so this stays out of the framework-agnostic `@sparq/client`.
 */
export function loadIntoStore(
  Store: WasmModule["Store"],
  text: string,
  format: string,
): WasmStore {
  return isDatasetFormat(format)
    ? Store.loadDataset(text, format)
    : Store.load(text, format);
}

/**
 * [OPUS-4.8] sq-17nw — serialises a store's WHOLE dataset (default graph PLUS every
 * named graph) to N-Quads, by selecting {@link ALL_QUADS_QUERY} and emitting one
 * triple/quad line per solution. Used to merge two stores format-agnostically when the
 * user chooses "add to current": concatenate both stores' N-Quads and re-load with
 * `Store.loadDataset(_, "nquads")` so the named graphs survive the round-trip.
 */
export function storeToNQuads(store: WasmStore): string {
  const json = store.query(ALL_QUADS_QUERY);
  const parsed = JSON.parse(json) as SparqlResults;
  return rowsToNQuads(parsed.results.bindings);
}

/**
 * [OPUS-4.8] sq-17nw — the TOTAL number of triples in a store across the default graph
 * and all named graphs. The wasm `store.size` getter counts the DEFAULT graph only, so it
 * under-reports a dataset; this counts the whole thing via {@link ALL_QUADS_QUERY} for an
 * accurate "N triples" badge. Falls back to `store.size` if the count query fails.
 */
export function datasetSize(store: WasmStore): number {
  try {
    const json = store.query(`SELECT (COUNT(*) AS ?n) WHERE { ${ALL_QUADS_BODY} }`);
    const parsed = JSON.parse(json) as SparqlResults;
    const n = termValue(parsed.results.bindings[0]?.n);
    const parsedN = n != null ? Number.parseInt(n, 10) : NaN;
    return Number.isFinite(parsedN) ? parsedN : store.size;
  } catch {
    return store.size;
  }
}

/**
 * [SONNET-4.6] sq-su1oe (#820) — a ROUGH estimate of the in-tab store's in-memory footprint,
 * in bytes, via the wasm `store.heapBytes()` binding. This is an in-memory (heap) figure for
 * the WASM triplestore, NOT a disk-usage figure: the site is a static, client-side
 * GitHub-Pages app with no server "disk", so we report what is genuinely measurable — the
 * store's heap footprint. Returns `null` if the binding is unavailable so callers can omit
 * the stat rather than show a fabricated number.
 */
export function storeFootprint(store: WasmStore): number | null {
  try {
    const bytes = store.heapBytes();
    return Number.isFinite(bytes) ? bytes : null;
  } catch {
    return null;
  }
}

/**
 * [OPUS-4.8] sq-oy1f.7 — the JSON-LD output modes offered for a CONSTRUCT /
 * DESCRIBE result graph. The three left-hand modes drive the wasm `Store.serialize`
 * binding's JSON-LD document forms (expanded / flattened / prefix-`@context`-compacted —
 * #900/#923); `"compact"` drives the SEPARATE `Store.serializeCompact` binding — the full
 * **W3C JSON-LD 1.1 Compaction Algorithm** against a caller-supplied `@context` (sq-oy1f.5,
 * #957). Both bindings live behind the engine's OPT-IN `serialize-rdf` feature, which the
 * site's `build:wasm` bundle enables (`--features shacl,jsonld,serialize-rdf,scs`).
 */
export type JsonLdMode = "expanded" | "flattened" | "compacted" | "compact";

/** The wasm `Store.serialize` JSON-LD document-form format string each mode selects. */
const JSONLD_SERIALIZE_FORMAT: Record<
  Exclude<JsonLdMode, "compact">,
  string
> = {
  expanded: "jsonld-expanded",
  flattened: "jsonld-flattened",
  compacted: "jsonld-compacted",
};

/**
 * [OPUS-4.8] sq-oy1f.7 — serialise a CONSTRUCT / DESCRIBE result graph (the engine's flat
 * N-Triples document) as JSON-LD via the wasm engine's OWN writer — never a TS reshaper.
 *
 * The result graph is loaded into an EPHEMERAL `Store` (so the serialise reflects exactly
 * the result triples, not the whole loaded dataset), then emitted in the chosen JSON-LD
 * form. For the three document forms this routes through `Store.serialize` (pretty,
 * two-space indent). For the `"compact"` mode it routes through `Store.serializeCompact`,
 * applying the full **W3C JSON-LD 1.1 Compaction Algorithm** against the caller's
 * `@context` JSON text — term definitions, `@vocab`, type/language/`@container` coercion,
 * `@reverse`, and `@id`/`@type` aliasing. A non-object / malformed `@context` is rejected
 * by the wasm binding with a clear `Error` (it never emits a silently-wrong document).
 *
 * `ntriples` is the engine's flat result document (the REPL already holds it). `context` is
 * the `@context` JSON text — required (and only used) when `mode === "compact"`.
 */
export async function serializeGraphAsJsonLd(
  ntriples: string,
  mode: JsonLdMode,
  context?: string,
): Promise<string> {
  const Store = await loadSparq();
  // N-Triples carry no named graphs, so the cheaper triple-only `load` is correct here.
  const store = loadIntoStore(Store, ntriples, "ntriples");
  if (mode === "compact") {
    // The full W3C JSON-LD 1.1 Compaction against the caller's `@context` (sq-oy1f.5).
    return store.serializeCompact(context ?? "{}", true, "  ");
  }
  // The expanded / flattened / prefix-compacted document forms (#900/#923). `abbreviate`
  // only affects the Turtle/TriG writers; the prefix-`@context` of `jsonld-compacted` is
  // built from the engine's default prefix registry (no caller prefix map passed here).
  return store.serialize(JSONLD_SERIALIZE_FORMAT[mode], true, "  ", true);
}
