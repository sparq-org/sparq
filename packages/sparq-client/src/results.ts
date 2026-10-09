// [OPUS-4.8] sq-x0kp — framework-agnostic SPARQL-results formatting for the /try REPL
// results panel (GUI MVP item 2 in research/gui-design.md §"Phase 1").
//
// WHY HERE. The GUI design record's load-bearing property is that the PURE part of the GUI
// "transfers" to any host (the Next.js site today, the proposed Tauri 2 webview later). The
// React result panel is host-shaped, but the LOGIC that turns a SPARQL 1.1 JSON results
// document into a typed table, a CSV/TSV export, or a pretty-printed raw view is pure data
// shaping with no DOM and no framework — so it lives in this shared `@sparq/client` package
// (alongside `formatTerm` / `streamQueryRows` / the SPARQL-JSON shapes) and both hosts draw
// the same cells. This file is the single source of truth for "how a result renders".
//
// SCOPE. It shapes the three answer shapes the lean wasm bundle returns through its query
// surface: a SELECT result (`Store.query` -> SPARQL-JSON with `head.vars` + bindings), an
// ASK result (the same document carrying a top-level `boolean`), and — for completeness so
// the panel has one place to ask "what shape is this?" — a discriminator over both. The
// CONSTRUCT/DESCRIBE (N-Triples) and EXPLAIN (plan text) shapes are already plain strings the
// engine returns, so they need no shaping here; the panel renders them verbatim.
//
// No performance claim is made anywhere here.

import type { SparqlResults, SparqlTerm, SparqlBinding } from "./index.js";
import { formatTerm, termValue } from "./index.js";

// ---------------------------------------------------------------------------
// Reading a SPARQL-JSON results document.
// ---------------------------------------------------------------------------

/** The variable names a SELECT result projects (its `head.vars`), in order. */
export function resultVars(results: SparqlResults): string[] {
  return results.head?.vars ?? [];
}

/** The solution rows of a SELECT result (each a var→term binding map), in order. */
export function resultRows(results: SparqlResults): SparqlBinding[] {
  return results.results?.bindings ?? [];
}

/**
 * Whether a parsed SPARQL-JSON document is an ASK answer — it carries a top-level
 * `boolean`. (A SELECT document has `head.vars` + `results.bindings` and no `boolean`.)
 */
export function isAskResult(results: SparqlResults): boolean {
  return typeof results.boolean === "boolean";
}

/** The boolean of an ASK answer, or `null` if the document is not an ASK result. */
export function askValue(results: SparqlResults): boolean | null {
  return typeof results.boolean === "boolean" ? results.boolean : null;
}

// ---------------------------------------------------------------------------
// The typed TABLE view (columns from the head vars, cells via formatTerm).
// ---------------------------------------------------------------------------

/** A SELECT result shaped as a table: the column headers plus one string row per solution. */
export interface ResultTable {
  /** The column headers — the projection's variable names, in `head.vars` order. */
  vars: string[];
  /**
   * One row per solution. Each row has exactly `vars.length` cells (columns in `vars`
   * order); an unbound projection variable in a solution renders as the empty string.
   */
  rows: string[][];
}

/**
 * [OPUS-4.8] sq-x0kp — shape a SELECT result set into a {@link ResultTable}: the columns are
 * the projected variables (in `head.vars` order) and each cell is the term rendered by the
 * shared {@link formatTerm} (so a URI shows as `<…>`, a typed literal keeps its `^^xsd:…`
 * suffix, a lang literal its `@lang`, and an unbound variable is the empty string). This is
 * the single extraction both the site table and the Tauri webview table draw from.
 *
 * Passing an ASK document yields an empty table (`vars: []`, `rows: []`): a table is not the
 * right view for a boolean, and the caller should branch on {@link isAskResult} first.
 */
export function extractTable(results: SparqlResults): ResultTable {
  if (isAskResult(results)) return { vars: [], rows: [] };
  const vars = resultVars(results);
  const rows = resultRows(results).map((binding) =>
    vars.map((v) => formatTerm(binding[v])),
  );
  return { vars, rows };
}

// ---------------------------------------------------------------------------
// CSV / TSV export (MVP: "table + raw SPARQL-JSON + N-Triples, plus CSV/TSV export").
// ---------------------------------------------------------------------------

/**
 * The RAW lexical value of a term for a data export, WITHOUT the display decoration
 * {@link formatTerm} adds (no `<>` around a URI, no surrounding quotes / `^^`/`@` on a
 * literal). A CSV/TSV consumer wants the value itself (`http://ex/a`, `42`, `Alice`), and the
 * cell-level quoting/escaping is handled by {@link csvCell} / {@link tsvCell}. An unbound
 * projection variable exports as an empty cell.
 */
function exportValue(t: SparqlTerm | undefined): string {
  return termValue(t) ?? "";
}

/**
 * One CSV cell, quoted per RFC 4180: a field containing a comma, double-quote, CR or LF is
 * wrapped in double-quotes with each embedded double-quote doubled. Other fields pass through
 * verbatim. (CRLF is the RFC line terminator; {@link resultsToCsv} joins rows with it.)
 */
export function csvCell(value: string): string {
  return /[",\r\n]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value;
}

/**
 * One TSV cell: tab/CR/LF are NOT representable inside a TSV field, so they are replaced with
 * a single space (the conventional lossless-enough TSV escape — TSV has no quoting). This
 * keeps every solution on exactly one line with one column per tab.
 */
export function tsvCell(value: string): string {
  return value.replace(/[\t\r\n]+/g, " ");
}

/**
 * [OPUS-4.8] sq-x0kp — export a SELECT result as CSV (RFC 4180): a header row of the
 * projected variable names, then one row per solution of the terms' RAW lexical values
 * ({@link exportValue}), each cell quoted by {@link csvCell}, rows joined with CRLF. An ASK
 * document (no projection) exports as the empty string. A result with columns but no
 * solutions exports just the header row. This is the "Download CSV" payload.
 */
export function resultsToCsv(results: SparqlResults): string {
  if (isAskResult(results)) return "";
  const vars = resultVars(results);
  const header = vars.map(csvCell).join(",");
  const body = resultRows(results).map((binding) =>
    vars.map((v) => csvCell(exportValue(binding[v]))).join(","),
  );
  return [header, ...body].join("\r\n");
}

/**
 * [OPUS-4.8] sq-x0kp — export a SELECT result as TSV: a tab-separated header row of the
 * projected variable names, then one tab-separated row per solution of the terms' RAW
 * lexical values ({@link exportValue}) sanitised by {@link tsvCell}, rows joined with LF. An
 * ASK document exports as the empty string; a result with columns but no solutions exports
 * just the header row. This is the "Download TSV" payload.
 */
export function resultsToTsv(results: SparqlResults): string {
  if (isAskResult(results)) return "";
  const vars = resultVars(results);
  const header = vars.map(tsvCell).join("\t");
  const body = resultRows(results).map((binding) =>
    vars.map((v) => tsvCell(exportValue(binding[v]))).join("\t"),
  );
  return [header, ...body].join("\n");
}

// ---------------------------------------------------------------------------
// The RAW SPARQL-JSON view (a pretty-printed copy of the engine's document).
// ---------------------------------------------------------------------------

/**
 * [OPUS-4.8] sq-x0kp — pretty-print a parsed SPARQL-JSON results document for the raw-JSON
 * toggle: a stable 2-space-indented re-serialisation of the EXACT document the engine
 * returned (`head` + `results`/`boolean`). Re-serialising the parsed object (rather than
 * threading the original string) guarantees the panel shows valid, canonically-indented JSON
 * regardless of how the engine spaced its output.
 */
export function formatSparqlJson(results: SparqlResults): string {
  return JSON.stringify(results, null, 2);
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-9w4t (#817) — PAGINATED result rendering + a bounded read-ahead page cache.
//
// WHY. Maintainer #817: serialising/rendering the WHOLE kept result is the main per-render
// overhead, but the user only ever LOOKS at one screenful. So the panel should render (and
// shape into display cells) only the VISIBLE slice, advancing on a page change, materialising
// a couple of pages ahead so the next ⏭ is instant. This is the RENDERING half of the bead —
// pure data shaping over the rows already streamed into JS, with no DOM and no framework, so
// it lives here in `@sparq/client` and the React panel just draws what it returns.
//
// SCOPE — what this is NOT. The bead's note is correct: the engine result path is currently
// MATERIALISED (the wasm `SolutionCursor` slices an already-evaluated `result.rows` —
// crates/sparq-wasm/src/lib.rs), so there is no PULL iterator to drive *demand-driven query
// EVALUATION* (computing the engine work only up to the current page). That LAZY-EVAL half is
// gated on a pull/iterator exec model in `sparq-engine` and is tracked separately (a follow-up
// bead + research/gui-design.md §A.4); this module paginates over the rows already in hand and
// makes no performance claim.
// ---------------------------------------------------------------------------

/** A single page of a paginated {@link ResultTable}: the slice plus where it sits. */
export interface ResultPage {
  /** The column headers (the whole result's `vars`, unchanged across pages). */
  vars: string[];
  /** The display-shaped rows for THIS page only (≤ `pageSize`). */
  rows: string[][];
  /** 0-based index of this page. */
  page: number;
  /** Rows per page this page was sliced at. */
  pageSize: number;
  /** 0-based index of the first row of this page within the whole result. */
  startRow: number;
  /** Total rows across the whole (kept) result — the denominator for "x–y of N". */
  totalRows: number;
  /** Total number of pages at this `pageSize` (≥ 1, even for an empty result). */
  totalPages: number;
}

/** Total number of pages `totalRows` rows split into, at `pageSize` rows per page (≥ 1). */
export function pageCount(totalRows: number, pageSize: number): number {
  if (pageSize <= 0) return 1;
  if (totalRows <= 0) return 1;
  return Math.ceil(totalRows / pageSize);
}

/** Clamp a (possibly out-of-range) 0-based page index into `[0, pageCount-1]`. */
export function clampPage(page: number, totalRows: number, pageSize: number): number {
  const last = pageCount(totalRows, pageSize) - 1;
  if (!Number.isFinite(page) || page < 0) return 0;
  return page > last ? last : Math.floor(page);
}

/**
 * [OPUS-4.8] sq-9w4t — slice ONE page of display-shaped cells out of a {@link ResultTable}.
 * The `page` is clamped into range (an out-of-range page yields the nearest valid page, never
 * an empty slice on a non-empty result). `pageSize ≤ 0` is treated as "one page of everything".
 * Only the `pageSize` rows of the requested page are shaped — the whole-table `table.rows` is
 * already display-shaped by {@link extractTable}, so this is a bounded slice, not a re-render of
 * every row.
 */
export function paginateTable(
  table: ResultTable,
  page: number,
  pageSize: number,
): ResultPage {
  const totalRows = table.rows.length;
  const effectiveSize = pageSize > 0 ? pageSize : Math.max(totalRows, 1);
  const totalPages = pageCount(totalRows, effectiveSize);
  const clamped = clampPage(page, totalRows, effectiveSize);
  const startRow = clamped * effectiveSize;
  return {
    vars: table.vars,
    rows: table.rows.slice(startRow, startRow + effectiveSize),
    page: clamped,
    pageSize: effectiveSize,
    startRow,
    totalRows,
    totalPages,
  };
}

/**
 * A bounded read-ahead cache over a paginated {@link ResultTable}. {@link PageCache.get} returns
 * the requested page (computing + memoising its slice on first access) and, as a side effect,
 * WARMS the next `readAhead` pages so a forward ⏭ is already materialised. The cache keeps at
 * most `1 + 2·readAhead` page slices live (the current page, `readAhead` ahead, `readAhead`
 * behind) and evicts the page furthest from the last-requested one — so paging through a large
 * result never holds more than that bounded window of shaped slices in memory at once.
 */
export interface PageCache {
  /** The page at `page` (clamped), warming the read-ahead window as a side effect. */
  get(page: number): ResultPage;
  /** How many page slices are currently materialised (for tests / introspection). */
  readonly size: number;
  /** Rows per page this cache paginates at. */
  readonly pageSize: number;
  /** Total pages at this page size (≥ 1). */
  readonly totalPages: number;
}

/**
 * [OPUS-4.8] sq-9w4t — build a {@link PageCache} over `table`. `pageSize` is the rows-per-page;
 * `readAhead` (default 2 — the maintainer's "~2 pages ahead") is how many pages forward to warm
 * on each `get`, and also bounds the retained window. The slices are computed lazily by
 * {@link paginateTable}, so constructing the cache shapes NOTHING — only the pages actually
 * visited (and their read-ahead) are ever materialised.
 */
export function createPageCache(
  table: ResultTable,
  pageSize: number,
  readAhead = 2,
): PageCache {
  const ahead = Math.max(0, Math.floor(readAhead));
  const total = pageCount(table.rows.length, pageSize > 0 ? pageSize : table.rows.length || 1);
  // Map page-index -> shaped slice, plus a parallel "last touched" tick for LRU-by-distance
  // eviction. `Map` preserves insertion order, but we evict by DISTANCE from the focus page
  // (not insertion order) so a back-and-forth around the focus keeps the hot window.
  const slices = new Map<number, ResultPage>();
  const cap = 1 + 2 * ahead;
  let focus = 0;

  const materialise = (p: number): ResultPage => {
    const existing = slices.get(p);
    if (existing) return existing;
    const slice = paginateTable(table, p, pageSize);
    // `paginateTable` clamps, so key by the CLAMPED page to avoid duplicate entries for the
    // same physical page (e.g. warming past the last page).
    const keyed = slices.get(slice.page);
    if (keyed) return keyed;
    slices.set(slice.page, slice);
    return slice;
  };

  const evictBeyondWindow = () => {
    if (slices.size <= cap) return;
    // Drop the entries furthest from the focus page until we are back within the window.
    const byDistance = [...slices.keys()].sort(
      (a, b) => Math.abs(b - focus) - Math.abs(a - focus),
    );
    for (const k of byDistance) {
      if (slices.size <= cap) break;
      slices.delete(k);
    }
  };

  return {
    get(page: number): ResultPage {
      const current = materialise(page);
      focus = current.page;
      // Warm the read-ahead window (forward, where a ⏭ goes next) AND one behind, so a ⏮ after
      // a ⏭ is also warm — all bounded by `cap` via the eviction below.
      for (let d = 1; d <= ahead; d += 1) {
        if (current.page + d < total) materialise(current.page + d);
        if (current.page - d >= 0) materialise(current.page - d);
      }
      evictBeyondWindow();
      return current;
    },
    get size() {
      return slices.size;
    },
    pageSize: pageSize > 0 ? pageSize : table.rows.length || 1,
    totalPages: total,
  };
}
