// [OPUS-4.8] sq-17nw — pure dataset helpers for the live SPARQL REPL.
//
// The REPL used to load EVERY uploaded document with `Store.load(text, format)`,
// which folds the named graphs of N-Quads / TriG into the default graph — so a
// `GRAPH <iri> { … }` query against an uploaded dataset returned nothing. These
// helpers let the REPL instead:
//   • pick the named-graph-preserving `Store.loadDataset` for the quad formats
//     (`isDatasetFormat`), and
//   • serialise a *whole* loaded dataset (default graph + every named graph) back
//     to N-Quads (`ALL_QUADS_QUERY` + `rowsToNQuads`) so the format-agnostic
//     "add to current" merge re-loads through `loadDataset` without losing graphs.
//
// They are framework-free and side-effect-free (no React, no DOM, no wasm) so they
// unit-test directly under `node --test`; the engine-level named-graph behaviour is
// covered by the Rust tests + js/test/store.test.mjs.

import { termToNTriples, type SparqlTerm } from "@sparq/client";

// [OPUS-4.8] sq-dvyi: JSON-LD can carry named graphs (`@graph` with an outer `@id`),
// so it joins nquads/trig as a dataset format routed through `loadDataset`.
/** Formats whose payload can carry named graphs (and so need `loadDataset`). */
const DATASET_FORMATS = new Set(["nquads", "trig", "jsonld"]);

/**
 * True for the quad-bearing RDF formats (`nquads` / `trig` / `jsonld`). Triple-only
 * formats (`turtle` / `ntriples`) have no named graphs, so the cheaper `Store.load` is
 * both correct and slightly faster for them.
 */
export function isDatasetFormat(format: string): boolean {
  return DATASET_FORMATS.has(format);
}

// [OPUS-4.8] sq-dvyi: the RDF formats the wasm engine's `load`/`loadDataset` accept,
// keyed by file extension. Lives here (not in the React component) so it is framework-
// free and unit-testable. JSON-LD is parsed engine-side (oxjsonld) in the lean wasm
// bundle. The component renders these as the upload/URL format choices.
export const FORMAT_OPTIONS: { value: string; label: string; exts: string[] }[] = [
  { value: "turtle", label: "Turtle (.ttl)", exts: ["ttl", "turtle"] },
  { value: "ntriples", label: "N-Triples (.nt)", exts: ["nt", "ntriples"] },
  { value: "nquads", label: "N-Quads (.nq)", exts: ["nq", "nquads"] },
  { value: "trig", label: "TriG (.trig)", exts: ["trig"] },
  { value: "jsonld", label: "JSON-LD (.jsonld)", exts: ["jsonld", "json-ld"] },
];

/** Best-effort engine format guess from a filename/URL; falls back to Turtle. */
export function guessFormat(name: string): string {
  const ext = name.split(/[?#]/)[0].split(".").pop()?.toLowerCase() ?? "";
  const hit = FORMAT_OPTIONS.find((f) => f.exts.includes(ext));
  return hit?.value ?? "turtle";
}

// Map a response `Content-Type` to an engine format, ignoring any `; charset=…`
// parameter. A served media type is more reliable than the URL extension for the
// auto-detect URL load (e.g. a JSON-LD endpoint with no `.jsonld` suffix), so the URL
// path prefers it when present. Returns `undefined` for an unknown/absent type so the
// caller can fall back to the extension guess.
const CONTENT_TYPE_FORMATS: Record<string, string> = {
  "text/turtle": "turtle",
  "application/n-triples": "ntriples",
  "application/n-quads": "nquads",
  "application/trig": "trig",
  "application/ld+json": "jsonld",
};

/** Engine format for a response `Content-Type`, or `undefined` if unrecognised. */
export function formatFromContentType(
  contentType: string | null,
): string | undefined {
  if (!contentType) return undefined;
  const mime = contentType.split(";")[0].trim().toLowerCase();
  return CONTENT_TYPE_FORMATS[mime];
}

/**
 * The graph-pattern that matches the WHOLE dataset: the default graph
 * (`{ ?s ?p ?o }`) UNION every named graph (`GRAPH ?g { ?s ?p ?o }`). Named-graph
 * solutions additionally bind `?g`; default-graph solutions leave it unbound.
 * Folding-only `{ ?s ?p ?o }` would miss the named graphs entirely — the bug this
 * fixes. Reused by the row enumeration and the COUNT-based total-size query.
 */
export const ALL_QUADS_BODY = "{ ?s ?p ?o } UNION { GRAPH ?g { ?s ?p ?o } }";

/** A SELECT that enumerates the whole dataset (see {@link ALL_QUADS_BODY}). */
export const ALL_QUADS_QUERY = `SELECT ?s ?p ?o ?g WHERE { ${ALL_QUADS_BODY} }`;

/** One solution row from {@link ALL_QUADS_QUERY}: bound `s`/`p`/`o`, optional `g`. */
type QuadRow = {
  s?: SparqlTerm;
  p?: SparqlTerm;
  o?: SparqlTerm;
  g?: SparqlTerm;
};

// [OPUS-4.8] sq-vfbm — query-form classification for the live REPL.
//
// The wasm Store exposes a different entry point per SPARQL form: `query` for
// SELECT/ASK (a solution table / boolean), `queryQuads` for the graph-valued forms
// CONSTRUCT/DESCRIBE (an N-Triples document), and `updateInPlace` for SPARQL Update
// (INSERT / DELETE / LOAD / graph management — mutates the store, returns nothing).
// EXPLAIN/EXPLAIN ANALYZE are a separate mode the user toggles, not a query form.
// The REPL dispatches on the form, so it must recognise it from the query text
// BEFORE handing it to the engine. This is a pure, framework-free classifier so it
// unit-tests directly; the engine itself remains the source of truth (a mis-classed
// query still surfaces the engine's own error).

export type QueryForm = "select" | "ask" | "construct" | "describe" | "update";

// The SPARQL Update keywords (SPARQL 1.1 §3.1 / §3.2). Any of these as the leading
// significant token means the request is an Update, not a query.
const UPDATE_KEYWORDS = [
  "INSERT",
  "DELETE",
  "LOAD",
  "CLEAR",
  "CREATE",
  "DROP",
  "ADD",
  "MOVE",
  "COPY",
  "WITH",
];

/**
 * Strips SPARQL comments (`# … <eol>`) and the leading PREFIX / BASE declarations
 * so the FIRST significant keyword can be read. Returns the remaining query text
 * upper-cased for keyword matching. Comments are stripped line-by-line; a `#` inside
 * an IRI or string literal in the prologue is not a concern because the prologue only
 * contains PREFIX/BASE declarations whose IRIs are angle-bracketed.
 */
function significantHead(sparql: string): string {
  const noComments = sparql
    .split("\n")
    .map((line) => {
      const hash = line.indexOf("#");
      return hash === -1 ? line : line.slice(0, hash);
    })
    .join("\n");
  // Drop leading PREFIX / BASE declarations (the prologue) repeatedly.
  let rest = noComments.trimStart();
  // PREFIX pfx: <iri>  |  BASE <iri>
  const prologue = /^(PREFIX\s+[^\s]*\s*<[^>]*>|BASE\s*<[^>]*>)\s*/i;
  while (prologue.test(rest)) {
    rest = rest.replace(prologue, "").trimStart();
  }
  return rest.toUpperCase();
}

/**
 * Classifies a SPARQL request into the engine entry point that handles it
 * ({@link QueryForm}). SELECT/ASK -> `query`; CONSTRUCT/DESCRIBE -> `queryQuads`;
 * any Update keyword -> `updateInPlace`. Unknown leading tokens default to `"select"`
 * so the engine's own parser produces the error, rather than this classifier guessing.
 */
export function classifyQueryForm(sparql: string): QueryForm {
  const head = significantHead(sparql);
  if (head.startsWith("ASK")) return "ask";
  if (head.startsWith("CONSTRUCT")) return "construct";
  if (head.startsWith("DESCRIBE")) return "describe";
  if (head.startsWith("SELECT")) return "select";
  for (const kw of UPDATE_KEYWORDS) {
    // Whole-word match: the keyword followed by whitespace, `{`, or end-of-string.
    if (
      head === kw ||
      head.startsWith(`${kw} `) ||
      head.startsWith(`${kw}\n`) ||
      head.startsWith(`${kw}{`)
    ) {
      return "update";
    }
  }
  return "select";
}

/** True for the graph-valued query forms answered by the wasm `queryQuads` export. */
export function isGraphForm(form: QueryForm): boolean {
  return form === "construct" || form === "describe";
}

// [OPUS-4.8] sq-xe4f — EXPLAIN / EXPLAIN ANALYZE support per query form.
//
// EXPLAIN/ANALYZE drive the *query* planner (`store.explain` / `store.explainAnalyze`),
// which only accept query forms — handing it a SPARQL Update yields an engine parse
// error ("expected CONSTRUCT"). So the REPL must NOT offer EXPLAIN/ANALYZE for an Update
// example. This pure predicate lets the ModeTabs component gray those tabs out (and lets
// `Repl` snap the mode back to "run" when an Update is loaded) instead of only degrading
// via the error panel after the fact. The engine stays the source of truth — a
// mis-classified query still surfaces the engine's own error.
export type RunMode = "run" | "explain" | "analyze";

/**
 * True when {@link mode} is a usable choice for {@link form}. `"run"` always is.
 * EXPLAIN / ANALYZE plan a *query* and reject SPARQL Update forms, so they are
 * unavailable for `"update"`.
 */
export function modeSupportsForm(mode: RunMode, form: QueryForm): boolean {
  if (mode === "run") return true;
  // EXPLAIN / EXPLAIN ANALYZE plan a *query*; SPARQL Update is not a query form.
  return form !== "update";
}

// [OPUS-4.8] sq-daru — dataset panel: per-graph triple counts.
//
// The panel enumerates the loaded dataset's graphs with a triple count each, so a user
// can see at a glance how the data is partitioned (default graph + every named graph).
// It reuses queries the engine already supports rather than a new Store method:
//
//   • NAMED-GRAPH stats — one row per named graph with its triple count. `GROUP BY ?g`
//     over `GRAPH ?g { ?s ?p ?o }` lists exactly the named graphs (the default graph is
//     NOT a GRAPH and so is excluded — it is counted separately).
//   • DEFAULT-GRAPH count — the default graph's triple count on its own.
//
// Both are pure strings here (no React/DOM/wasm) so they are unit-testable; the parser
// `parseGraphStats` turns the two SPARQL-JSON documents into the typed rows the panel
// renders, and is itself pure + framework-free.

/** The named-graph stats query: one solution per named graph, `?g` = graph IRI,
 *  `?c` = its triple count. The default graph is not a GRAPH, so it is excluded. */
export const NAMED_GRAPH_STATS_QUERY =
  "SELECT ?g (COUNT(*) AS ?c) WHERE { GRAPH ?g { ?s ?p ?o } } GROUP BY ?g ORDER BY ?g";

/** The default-graph triple count (a single solution binding `?c`). */
export const DEFAULT_GRAPH_COUNT_QUERY =
  "SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }";

/** One row of the dataset panel: a graph and its triple count. `iri` is `null` for the
 *  default graph (it has no IRI) and the graph IRI for a named graph. */
export interface GraphStat {
  /** The named-graph IRI, or `null` for the default graph. */
  iri: string | null;
  /** True for the default graph (always present, even when empty). */
  isDefault: boolean;
  /** The triple count in this graph. */
  count: number;
}

/** A structural subset of a parsed SPARQL-JSON results document — just what
 *  {@link parseGraphStats} reads. Lets the parser stay framework-free and avoids a hard
 *  dependency on the full `SparqlResults` shape for the (pure) unit tests. */
export interface SparqlResultsLike {
  results?: {
    bindings?: Array<Record<string, { value?: string } | undefined>>;
  };
}

/** Parses a COUNT solution's `?c` value to a non-negative integer, or `0` if absent /
 *  unparseable (a robust default so the panel never shows `NaN`). */
function countFromBinding(value: string | undefined): number {
  if (value == null) return 0;
  const n = Number.parseInt(value, 10);
  return Number.isFinite(n) && n >= 0 ? n : 0;
}

/**
 * Turns the two SPARQL-JSON documents — {@link DEFAULT_GRAPH_COUNT_QUERY} and
 * {@link NAMED_GRAPH_STATS_QUERY} — into the typed {@link GraphStat} rows the dataset
 * panel renders. The default graph is ALWAYS the first row (even with a count of 0, so
 * the panel is honest about an empty default graph); the named graphs follow in the
 * query's order. Malformed/absent counts default to 0 rather than throwing, so a partial
 * result still renders. Pure + framework-free (takes already-parsed JSON), so it
 * unit-tests directly.
 */
export function parseGraphStats(
  defaultCount: SparqlResultsLike,
  namedStats: SparqlResultsLike,
): GraphStat[] {
  const defaultBinding = defaultCount.results?.bindings?.[0];
  const stats: GraphStat[] = [
    {
      iri: null,
      isDefault: true,
      count: countFromBinding(defaultBinding?.c?.value),
    },
  ];
  for (const row of namedStats.results?.bindings ?? []) {
    const iri = row.g?.value;
    // A named-graph row must bind ?g; skip any defensive mis-shape.
    if (iri == null) continue;
    stats.push({ iri, isDefault: false, count: countFromBinding(row.c?.value) });
  }
  return stats;
}

/**
 * Serialises {@link ALL_QUADS_QUERY} solution rows to an N-Quads document: a triple
 * line (`s p o .`) when `?g` is unbound (default graph), a quad line
 * (`s p o g .`) when `?g` is bound (a named graph). Re-loading the result with
 * `Store.loadDataset(_, "nquads")` reconstructs the dataset with its named graphs
 * intact. Rows missing any of `s`/`p`/`o` are skipped defensively.
 */
export function rowsToNQuads(rows: QuadRow[]): string {
  const lines: string[] = [];
  for (const row of rows) {
    if (!row.s || !row.p || !row.o) continue;
    const spo = `${termToNTriples(row.s)} ${termToNTriples(row.p)} ${termToNTriples(row.o)}`;
    lines.push(row.g ? `${spo} ${termToNTriples(row.g)} .` : `${spo} .`);
  }
  return lines.join("\n");
}
