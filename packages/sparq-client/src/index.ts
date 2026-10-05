// [OPUS-4.8] sq-2e93 / sq-jpki — framework-agnostic sparq WASM client.
//
// This is the framework-agnostic entry point for the sparq-wasm `Store` surface plus the
// loaders / query helpers around it. It exists to kill the drift liability the GUI design
// record (`research/gui-design.md` §0 / §4) flags: the WASM `Store` TS interface was
// hand-redeclared inside `site/src/lib/sparq-wasm.ts`, so any future GUI would have become a
// THIRD hand-copy. Both the Next.js site and the (proposed) Tauri 2 GUI now consume this
// single source.
//
// [OPUS-4.8] sq-jpki — the `Store` surface here is NO LONGER hand-redeclared: `WasmStore` /
// `WasmSolutionCursor` / `WasmStoreCtor` are aliases over the wasm-pack-GENERATED `Store` /
// `SolutionCursor` classes (re-exported from `./generated/sparq_wasm.js`, the tracked verbatim
// build output). `npm run check:wasm-types` keeps that tracked copy byte-identical to a fresh
// `js/wasm` build, so the mirror cannot reappear and the surface cannot drift from the engine
// binding (§4's end-state). Everything BELOW the type re-export — the loaders, the SPARQL-JSON
// / SHACL render shapes, the `match()`/cursor helpers — is genuine framework-agnostic code.
//
// It is deliberately FRAMEWORK-AGNOSTIC: no React, no Next.js, no DOM beyond the two
// globals the wasm-pack glue needs at load time (`window.location`, dynamic `import()`).
// Everything here ports unchanged to any GUI framework the maintainer later prefers — the
// design's load-bearing "part (A) transfers" property.
//
// It does NOT re-implement the engine. `loadSparq()` fetches the same wasm-pack glue the
// site already syncs into `public/wasm/` (the lean `@sparq-org/sparq` bundle, built with
// `--features shacl,jsonld`), so this stays a thin, honest wrapper. No performance claim is
// made anywhere here (this repo's work box is non-canonical); a caller MAY time a single
// query with `performance.now()` and label it as a measured per-query latency, but must
// never bake in a benchmark number.

// ---------------------------------------------------------------------------
// The WASM `Store` surface — re-exported from the wasm-pack GENERATED d.ts.
// ---------------------------------------------------------------------------
//
// [OPUS-4.8] sq-jpki — the hand-redeclared `WasmStore` / `WasmSolutionCursor` /
// `WasmStoreCtor` mirror that used to live here is DELETED. `research/gui-design.md`
// §0/§4's end-state ("kill the hand-redeclared `WasmStore` drift") is now realised: these
// names are aliases over the wasm-pack-generated `Store` / `SolutionCursor` classes — the
// SINGLE source of truth. The generated d.ts is the verbatim build output for the site's
// `--features shacl,jsonld` bundle, tracked at `./generated/sparq_wasm.d.ts` (the live
// output in `js/wasm/` is git-ignored, so the bare-package typecheck has no other type
// source); `npm run check:wasm-types` asserts the tracked copy stays byte-identical to a
// freshly built `js/wasm/sparq_wasm.d.ts`, so the mirror cannot silently reappear and the
// surface cannot drift from the engine binding. A `.js` specifier resolves to the sibling
// `.d.ts` under `moduleResolution: "bundler"`; this is a type-only import (nothing emitted).

import type {
  Store as GeneratedStore,
  SolutionCursor as GeneratedSolutionCursor,
  InitInput,
  InitOutput,
} from "./generated/sparq_wasm.js";

/**
 * A forward-only cursor over a SELECT result's solution rows — the wasm-pack-generated
 * `SolutionCursor` ({@link WasmStore.queryCursor}). Each {@link WasmSolutionCursor.next}
 * yields the next BATCH of up to `batchSize` solutions as a SELF-CONTAINED SPARQL 1.1 JSON
 * document (so it can be `JSON.parse`d on its own), or `undefined` once every solution has
 * been yielded. The consumer never holds more than one batch, so peak JS memory is bounded
 * by `batchSize` rather than by the whole result. `vars`, `rowCount` and `batchSize`
 * introspect the (wasm-side materialised) result; `free` releases the cursor (call it if you
 * stop early). This is an alias over the generated class, not a hand re-declaration.
 */
export type WasmSolutionCursor = GeneratedSolutionCursor;

/**
 * The sparq-wasm `Store` surface, as exposed across the wasm-bindgen boundary — an alias
 * over the wasm-pack-GENERATED `Store` class (the single source of truth; see the header).
 * It carries the full generated surface — `query` / `queryQuads` / `updateInPlace` /
 * `explain` / `explainAnalyze` / `count` / `ask` / `queryCursor` / `applyDelta` / `size`,
 * plus the broader generated set (`askWithMaxRows`, `heapBytes`, `loadCompressed`,
 * `queryChunks`, `queryQuadsChunks`, `update`) and the SHACL `validate(data, shapes, format)`
 * binding the site's `--features shacl,jsonld` bundle emits.
 */
export type WasmStore = GeneratedStore;

/**
 * The static `Store` constructor surface (the named-graph load modes `load` / `loadDataset`,
 * plus the generated `loadCompressed`). This is the STATIC side of the generated `Store`
 * class (`typeof Store`), so {@link loadSparq}'s resolved value exposes the factory methods a
 * caller invokes as `Store.load(...)` / `Store.loadDataset(...)`.
 */
export type WasmStoreCtor = typeof GeneratedStore;

/**
 * The wasm-pack ESM module surface this client loads at runtime: the generated default
 * init function (it instantiates the wasm module) plus the generated `Store` class. The
 * loader calls `default({ module_or_path })` and reads `.Store`; both are taken from the
 * generated d.ts so this surface tracks the bundle.
 */
export interface WasmModule {
  default: (
    module_or_path?:
      | { module_or_path: InitInput | Promise<InitInput> }
      | InitInput
      | Promise<InitInput>,
  ) => Promise<InitOutput>;
  Store: WasmStoreCtor;
}

// ---------------------------------------------------------------------------
// SPARQL 1.1 JSON + SHACL report shapes (for rendering).
// ---------------------------------------------------------------------------

export interface SparqlTerm {
  type: "uri" | "literal" | "bnode";
  value: string;
  datatype?: string;
  "xml:lang"?: string;
}

export interface SparqlResults {
  head: { vars: string[] };
  results: { bindings: Record<string, SparqlTerm>[] };
  boolean?: boolean;
}

/** One row of a SELECT result (a variable → term binding map). */
export type SparqlBinding = SparqlResults["results"]["bindings"][number];

/**
 * One SHACL validation result, the per-violation record the wasm `Store.validate` binding
 * emits (a drop-in for `rdf-validate-shacl`'s results). `focusNode`/`value`/`sourceShape`
 * are N-Triples term strings; `path` is a SHACL Turtle path expression;
 * `sourceConstraintComponent`/`severity` are full IRIs. `path`/`value`/`message` are `null`
 * when the result carries none.
 */
export interface ShaclResult {
  focusNode: string;
  path: string | null;
  value: string | null;
  sourceShape: string;
  sourceConstraintComponent: string;
  severity: string;
  message: string | null;
}

/**
 * A SHACL validation report. `conforms` counts EVERY result regardless of severity (the
 * W3C-suite notion); `results` is the per-violation list.
 */
export interface ShaclReport {
  conforms: boolean;
  results: ShaclResult[];
}

// ---------------------------------------------------------------------------
// The runtime loader (the single cold-start path).
// ---------------------------------------------------------------------------

/**
 * Configuration for {@link loadSparq}. `basePath` is the URL prefix under which the
 * wasm-pack glue (`sparq_wasm.js`) and the wasm binary (`sparq_wasm_bg.wasm`) are served —
 * e.g. `"/sparq"` on GitHub Pages, `""` for a local dev server, or a `tauri://`/`file://`
 * origin for the desktop GUI. Defaulted from `NEXT_PUBLIC_BASE_PATH` (falling back to
 * `"/sparq"`) when not given, so the site keeps its current behaviour with no call-site
 * change.
 */
export interface LoadSparqOptions {
  basePath?: string;
}

// ---------------------------------------------------------------------------
// [SONNET-4.6] sq-b66fc — wasm-manifest.json resolution.
//
// The sync scripts emit public/wasm/wasm-manifest.json mapping each logical runtime
// filename (e.g. "sparq_wasm.js") to its content-hashed copy (e.g.
// "sparq_wasm-a1b2c3d4ef56.js"). The loader resolves asset URLs through this manifest
// so every redeploy to GitHub Pages (~10 min max-age) gets a fresh content-addressed
// URL — eliminating stale-glue / new-wasm mismatches. When the manifest is absent
// (dev mode / local disk / Tauri), we fall back to the unhashed URL transparently.
// ---------------------------------------------------------------------------

/**
 * Maps logical wasm runtime filename → content-hashed filename (just the bare filename,
 * not the full path). For tier-b bundles the key includes the subdirectory prefix,
 * e.g. `"reason/sparq_reason_wasm.js"`.
 *
 * @example
 * ```json
 * {
 *   "sparq_wasm.js": "sparq_wasm-a1b2c3d4ef56.js",
 *   "sparq_wasm_bg.wasm": "sparq_wasm_bg-7890abcdef01.wasm",
 *   "reason/sparq_reason_wasm.js": "sparq_reason_wasm-deadbeef1234.js"
 * }
 * ```
 */
export interface WasmManifest {
  [logicalName: string]: string;
}

// Module-level cache: basePath → resolved manifest (or null on 404/fetch failure).
const manifestCache = new Map<string, WasmManifest | null>();

/**
 * [SONNET-4.6] sq-b66fc — resolve a wasm asset URL through the content-hash manifest.
 *
 * Loads `${basePath}/wasm/wasm-manifest.json` once per basePath and caches the result.
 * If the manifest has an entry for `logicalName`, returns the full content-addressed URL;
 * otherwise falls back to the unhashed URL (dev mode / Tauri / absent manifest).
 *
 * In a non-browser environment (SSR / Node), fetch may be unavailable — falls back
 * directly to the unhashed URL without attempting a network call.
 *
 * @param logicalName  The bare filename (or `"subdir/filename"` for tier-b bundles),
 *                     e.g. `"sparq_wasm.js"` or `"reason/sparq_reason_wasm.js"`.
 * @param basePath     The URL prefix (same as {@link LoadSparqOptions.basePath}).
 *                     Defaults to the same `defaultBasePath()` the loader uses.
 */
export async function resolveWasmAsset(
  logicalName: string,
  basePath?: string,
): Promise<string> {
  const base = basePath ?? defaultBasePath();
  // Non-browser environment: skip the network fetch and fall back immediately.
  if (typeof fetch === "undefined") {
    return `${base}/wasm/${logicalName}`;
  }

  if (!manifestCache.has(base)) {
    const manifestUrl = `${base}/wasm/wasm-manifest.json`;
    try {
      const resp = await fetch(manifestUrl);
      if (!resp.ok) {
        // 404 or any non-2xx → treat as "no manifest" (dev mode / Tauri).
        manifestCache.set(base, null);
      } else {
        manifestCache.set(base, (await resp.json()) as WasmManifest);
      }
    } catch {
      // Network failure or JSON parse error → silent fallback.
      manifestCache.set(base, null);
    }
  }

  const manifest = manifestCache.get(base) ?? null;
  return resolveWasmAssetWithManifest(logicalName, manifest, base);
}

/**
 * [SONNET-4.6] sq-b66fc — pure resolution helper: resolve a wasm asset URL given an
 * already-loaded manifest (or `null` when the manifest is absent). This function is
 * exported so tests can call it directly without any network mocking.
 *
 * @param logicalName  The bare filename (or `"subdir/filename"` for tier-b bundles).
 * @param manifest     A pre-loaded {@link WasmManifest}, or `null` to force the fallback.
 * @param basePath     The URL prefix.
 */
export function resolveWasmAssetWithManifest(
  logicalName: string,
  manifest: WasmManifest | null,
  basePath: string,
): string {
  // Determine the subdirectory and bare filename from the logical name.
  const lastSlash = logicalName.lastIndexOf("/");
  const subdir = lastSlash === -1 ? "" : logicalName.slice(0, lastSlash + 1);
  const bareName = lastSlash === -1 ? logicalName : logicalName.slice(lastSlash + 1);

  const hashedFilename = manifest?.[logicalName];
  const resolvedName = hashedFilename ?? bareName;
  return `${basePath}/wasm/${subdir}${resolvedName}`;
}

function defaultBasePath(): string {
  // The Next.js site sets basePath '/sparq'; mirror it for the runtime asset URLs.
  // Empty in local `next dev` without basePath, '/sparq' on Pages. A GUI host passes its
  // own (e.g. "") explicitly via {@link LoadSparqOptions}.
  const env =
    typeof process !== "undefined" ? process.env?.NEXT_PUBLIC_BASE_PATH : undefined;
  return env ?? "/sparq";
}

let modulePromise: Promise<WasmModule> | null = null;
let modulePromiseBase: string | null = null;

/**
 * Loads + initialises the wasm engine once; subsequent calls reuse it.
 *
 * The fetch + compile + instantiate is the expensive cold start. {@link prewarmSparq}
 * kicks this off eagerly on route mount so the first query pays no cold start. The glue is
 * imported with `webpackIgnore` so it never enters a bundler's graph — it is fetched as a
 * plain ESM module at runtime, and the wasm URL is passed explicitly (the glue's own
 * `new URL('./sparq_wasm_bg.wasm', import.meta.url)` resolution is bypassed). If the
 * resolved base path changes between calls (e.g. site vs GUI), the module is re-loaded.
 *
 * [SONNET-4.6] sq-b66fc — glue and wasm paths are now resolved through the content-hash
 * manifest via {@link resolveWasmAsset} so redeployments to GitHub Pages get fresh
 * content-addressed URLs, eliminating stale-glue / new-wasm mismatches.
 */
export async function loadSparq(opts: LoadSparqOptions = {}): Promise<WasmStoreCtor> {
  const base = opts.basePath ?? defaultBasePath();
  if (!modulePromise || modulePromiseBase !== base) {
    modulePromiseBase = base;
    modulePromise = (async () => {
      // [SONNET-4.6] sq-b66fc — resolve through the manifest so a redeployed GitHub Pages
      // site always fetches the content-addressed glue+wasm pair.
      const gluePath = await resolveWasmAsset("sparq_wasm.js", base);
      const wasmPath = await resolveWasmAsset("sparq_wasm_bg.wasm", base);
      // webpackIgnore keeps the glue out of the bundle: it's fetched as a plain ESM
      // module from the static asset root at runtime.
      const mod = (await import(/* webpackIgnore: true */ gluePath)) as WasmModule;
      const origin =
        typeof window !== "undefined" ? window.location.origin : undefined;
      await mod.default({
        module_or_path: origin ? new URL(wasmPath, origin) : wasmPath,
      });
      return mod;
    })();
    modulePromise.catch(() => {
      modulePromise = null; // allow retry on transient failure
      modulePromiseBase = null;
    });
  }
  const mod = await modulePromise;
  return mod.Store;
}

/**
 * Eagerly pre-warm the wasm engine (fetch + compile + instantiate) without blocking
 * render. Safe to call on route mount and to call repeatedly — it shares the single
 * {@link loadSparq} promise, so the cold start happens at most once. Returns a promise that
 * resolves when the engine is ready (or rejects on a load failure, which resets the cache
 * so a later call can retry).
 *
 * Note this kicks the fetch off IMMEDIATELY (synchronously from the call site). For the
 * "load async AFTER the page paints" posture the maintainer asked for (#935 / #981 — keep
 * the ~300 kB wasm off the critical path so the page is interactive FIRST), prefer
 * {@link prewarmSparqWhenIdle}, which defers the fetch to the next browser-idle slot.
 */
export function prewarmSparq(opts: LoadSparqOptions = {}): Promise<unknown> {
  return loadSparq(opts);
}

/** The handle {@link prewarmSparqWhenIdle} returns so a caller can cancel a not-yet-fired
 * idle prewarm (e.g. a React effect cleanup on unmount). Calling it before the idle slot
 * fires prevents the wasm fetch from being kicked off; calling it after is a no-op. */
export interface IdlePrewarmHandle {
  /** Cancel the pending idle prewarm if it has not fired yet. */
  cancel(): void;
}

/**
 * [OPUS-4.8] sq-4296 (#935 / #981) — pre-warm the wasm engine LAZILY, on the next
 * browser-idle slot, so the ~300 kB engine wasm never competes with the initial page paint.
 *
 * This is the "load async after the page loads" mechanism: instead of firing the wasm fetch
 * synchronously while a wasm-using component mounts (which the maintainer flagged as blocking
 * page load), it schedules the fetch via `requestIdleCallback` — so the browser finishes
 * layout / paint / first-input readiness FIRST, then fetches + instantiates the wasm in the
 * background. The page is interactive before the wasm finishes loading; the (memoised)
 * {@link loadSparq} promise is reused, so any later on-demand `await loadSparq()` (e.g. the
 * first "Run query") simply joins the in-flight or already-resolved load — never a second
 * cold start, never a call into an uninitialised wasm.
 *
 * `requestIdleCallback` is feature-detected: where it is unavailable (Safari < 16.4, the SSR
 * pass, the static-export prerender) it falls back to a short `setTimeout(…, 1)`, which still
 * yields to the paint before fetching. The optional `timeout` (default 2000 ms) caps how long
 * the idle wait may stretch under sustained main-thread work, so the engine still warms on a
 * busy page. Returns an {@link IdlePrewarmHandle} whose `cancel()` unschedules a not-yet-fired
 * prewarm — pass it to a React effect cleanup so an unmount before the idle slot does no work.
 *
 * `onReady` / `onError` are optional readiness callbacks the (memoised) load resolves into, so
 * a component can flip its engine pill to "ready" / "error" without re-importing `loadSparq`.
 */
export function prewarmSparqWhenIdle(
  opts: LoadSparqOptions & {
    timeout?: number;
    onReady?: () => void;
    onError?: (err: unknown) => void;
  } = {},
): IdlePrewarmHandle {
  const { timeout = 2000, onReady, onError, ...loadOpts } = opts;
  let cancelled = false;

  const fire = (): void => {
    if (cancelled) return;
    loadSparq(loadOpts).then(
      () => {
        if (!cancelled) onReady?.();
      },
      (err) => {
        if (!cancelled) onError?.(err);
      },
    );
  };

  // Schedule on the next idle slot so the page paints / becomes interactive first. Outside a
  // browser (SSR / static prerender) `requestIdleCallback` is undefined; a short timeout
  // there still yields to any paint before the fetch is kicked off.
  type RIC = (cb: () => void, opts?: { timeout?: number }) => number;
  type CIC = (handle: number) => void;
  const ric =
    typeof window !== "undefined"
      ? (window as unknown as { requestIdleCallback?: RIC }).requestIdleCallback
      : undefined;
  const cic =
    typeof window !== "undefined"
      ? (window as unknown as { cancelIdleCallback?: CIC }).cancelIdleCallback
      : undefined;

  if (typeof ric === "function") {
    const id = ric(fire, { timeout });
    return {
      cancel() {
        cancelled = true;
        if (typeof cic === "function") cic(id);
      },
    };
  }

  const id = setTimeout(fire, 1);
  return {
    cancel() {
      cancelled = true;
      clearTimeout(id);
    },
  };
}

// ---------------------------------------------------------------------------
// RDF/JS `match()`-style helpers (framework-agnostic; no dataset-format coupling).
// ---------------------------------------------------------------------------

// The RDF/JS `match()` term shape: `null` (and the empty string in a UI) is a wildcard; a
// non-empty string is an N-Triples term (`<iri>`, `"literal"`, `"v"^^<dt>` …) inlined
// verbatim into the generated SELECT.
export type MatchTerm = string | null;

/** One position of a generated triple pattern: a constant term or a fresh variable. */
function patternPosition(term: MatchTerm, variable: string): string {
  const t = term?.trim();
  return t ? t : `?${variable}`;
}

/** The generated SELECT a `match()`/`countQuads()` lookup runs (shared so both agree). */
function matchSelect(
  subject: MatchTerm,
  predicate: MatchTerm,
  object: MatchTerm,
  graph: MatchTerm,
): string {
  const s = patternPosition(subject, "s");
  const p = patternPosition(predicate, "p");
  const o = patternPosition(object, "o");
  const triple = `${s} ${p} ${o}`;
  const g = graph?.trim();
  const where = g
    ? `{ GRAPH ${g} { ${triple} } }`
    : `{ { ${triple} } UNION { GRAPH ?g { ${triple} } } }`;
  return `SELECT * WHERE ${where}`;
}

/**
 * RDF/JS `match()`-style quad lookup over a wasm {@link WasmStore}, the same generated-SELECT
 * strategy `@sparq-org/sparq`'s `SparqStore.match` uses: each of subject/predicate/object/graph
 * is either a wildcard (a fresh `?s`/`?p`/`?o`/`?g` variable) or an inlined constant
 * N-Triples term. The graph wildcard spans the default graph AND every named graph. Returns
 * the matching rows as SPARQL-JSON bindings.
 */
export function matchQuads(
  store: WasmStore,
  subject: MatchTerm = null,
  predicate: MatchTerm = null,
  object: MatchTerm = null,
  graph: MatchTerm = null,
): SparqlBinding[] {
  const json = store.query(matchSelect(subject, predicate, object, graph));
  return (JSON.parse(json) as SparqlResults).results.bindings;
}

/**
 * `match(…).length` without materialising the matched terms, via the engine's index-level
 * {@link WasmStore.count} (the count of a generated SELECT over the same pattern
 * {@link matchQuads} builds). This is `countQuads()` in `@sparq-org/sparq`.
 */
export function countQuads(
  store: WasmStore,
  subject: MatchTerm = null,
  predicate: MatchTerm = null,
  object: MatchTerm = null,
  graph: MatchTerm = null,
): number {
  return store.count(matchSelect(subject, predicate, object, graph));
}

/** One batch a {@link streamQueryRows} pull surfaced: the rows plus the running totals. */
export interface CursorBatch {
  /** 1-based index of this batch. */
  index: number;
  /** The rows in THIS batch (at most `batchSize`). */
  rows: SparqlBinding[];
  /** Total rows yielded so far, across all batches pulled. */
  cumulative: number;
}

/**
 * Stream a SELECT query's rows one BATCH at a time through the wasm `queryCursor` cursor
 * (mirrors `@sparq-org/sparq`'s `queryBindingsStream`): pull a batch of up to `batchSize`
 * self-contained SPARQL-JSON rows, hand it to `onBatch`, drop it, pull the next — so the
 * consumer never holds more than one batch. Returns the cursor's introspection (`vars`,
 * `rowCount`, `batchSize`) once drained. The wasm cursor is always freed, even if `onBatch`
 * throws. An empty result yields exactly one empty batch.
 */
export function streamQueryRows(
  store: WasmStore,
  sparql: string,
  batchSize: number,
  onBatch?: (batch: CursorBatch) => void,
): { vars: string[]; rowCount: number; batchSize: number } {
  const cursor = store.queryCursor(sparql, batchSize);
  const meta = {
    vars: cursor.vars(),
    rowCount: cursor.rowCount(),
    batchSize: cursor.batchSize(),
  };
  try {
    let index = 0;
    let cumulative = 0;
    for (;;) {
      const chunk = cursor.next();
      if (chunk === undefined) break;
      const rows = (JSON.parse(chunk) as SparqlResults).results.bindings;
      cumulative += rows.length;
      onBatch?.({ index: ++index, rows, cumulative });
    }
  } finally {
    cursor.free();
  }
  return meta;
}

// ---------------------------------------------------------------------------
// SHACL validation (stateless one-shot) + term rendering.
// ---------------------------------------------------------------------------

/**
 * Validate an RDF **data** document against a SHACL **shapes** document, entirely in-tab via
 * the shacl-enabled wasm bundle. Both arguments use the syntaxes the loader accepts. A clear
 * error is thrown if the loaded bundle was built without the `shacl` feature (no `validate`
 * binding), so a caller can distinguish "no SHACL in this bundle" from a parse error. The
 * optional `loadOpts` is forwarded to {@link loadSparq} so a GUI host can pass its own
 * `basePath`.
 */
export async function sparqShaclValidate(
  data: string,
  shapes: string,
  format = "turtle",
  loadOpts: LoadSparqOptions = {},
): Promise<ShaclReport> {
  const Store = await loadSparq(loadOpts);
  // `validate` is a stateless one-shot, but it is exposed as an instance method on the wasm
  // `Store`, so we need any receiver. An empty store is the cheapest.
  const store = Store.load("", format);
  // The tracked generated d.ts is the site's `--features shacl,jsonld` bundle, so `validate`
  // is type-declared as present — but the bundle actually LOADED at runtime is decided by the
  // asset URL, not by this type, and a lean (no-`shacl`) bundle omits the binding. Keep the
  // defensive runtime check via a possibly-undefined view so a lean bundle yields a clear
  // error rather than a `store.validate is not a function` crash.
  const validate = (store as { validate?: WasmStore["validate"] }).validate;
  if (typeof validate !== "function") {
    throw new Error(
      "This wasm bundle was built without the SHACL feature (no validate binding). " +
        "Rebuild sparq-wasm with --features shacl.",
    );
  }
  // [OPUS-4.8] sq-800o — LOST-RECEIVER fix. The line above only EXISTENCE-checks a detached
  // method reference (so a lean, no-`shacl` bundle still yields the clear error above). It must
  // NOT be the thing we CALL: wasm-bindgen's `Store.validate` body does
  // `wasm.store_validate(this.__wbg_ptr, …)`, so invoking the detached `validate(…)` runs with
  // `this === undefined` and throws `Cannot read properties of undefined (reading '__wbg_ptr')`.
  // Invoke it BOUND to the `store` receiver so `this.__wbg_ptr` resolves to the live store.
  return JSON.parse(store.validate(data, shapes, format)) as ShaclReport;
}

/**
 * [OPUS-4.8] sq-pyn7 (#796) — parse a **SHACL Compact Syntax** (SCS) document into a SHACL
 * **shapes graph**, returned as pretty Turtle, entirely in-tab via the `scs`-enabled wasm
 * bundle. This is the SCS *input* counterpart of the playground's serialiser
 * (`shapesToCompact`, the Turtle → SCS *display* direction): it takes compact text and gives
 * back the Turtle shapes graph that {@link sparqShaclValidate} then consumes — so a user can
 * author shapes in the terser compact notation and validate data against them unchanged.
 *
 * Like {@link sparqShaclValidate} this is a stateless one-shot exposed as an instance method
 * on the wasm `Store`, so it needs any receiver — an empty store is the cheapest. The optional
 * `base` resolves relative IRIs in the SCS document; `loadOpts` is forwarded to
 * {@link loadSparq} so a GUI host can pass its own `basePath`.
 *
 * A clear error is thrown if the loaded bundle was built without the `scs` feature (no
 * `parseShaclCompact` binding) — the lean SPARQL REPL bundle omits it — so a caller can
 * distinguish "no SCS in this bundle" from an SCS *parse* error (the latter is a `JsError`
 * carrying the parser's message + 1-based line).
 */
export async function sparqParseShaclCompact(
  text: string,
  base?: string,
  loadOpts: LoadSparqOptions = {},
): Promise<string> {
  const Store = await loadSparq(loadOpts);
  // Stateless one-shot, but exposed as an instance method — the empty store is just the
  // receiver (the binding does not consult its triples).
  const store = Store.load("", "turtle");
  // Defensive lean-bundle check: the tracked generated d.ts is the site's `scs`-enabled
  // bundle, so `parseShaclCompact` is type-declared as present, but the bundle actually LOADED
  // at runtime is decided by the asset URL. A lean bundle omits the binding, so probe a
  // possibly-undefined view first to yield a clear error rather than a "not a function" crash.
  const parse = (store as { parseShaclCompact?: WasmStore["parseShaclCompact"] })
    .parseShaclCompact;
  if (typeof parse !== "function") {
    throw new Error(
      "This wasm bundle was built without the SHACL Compact Syntax feature (no " +
        "parseShaclCompact binding). Rebuild sparq-wasm with --features scs.",
    );
  }
  // Invoke BOUND to the `store` receiver (the same lost-receiver fix as `validate` above:
  // a detached `parse(…)` would run with `this === undefined` and throw on `this.__wbg_ptr`).
  return store.parseShaclCompact(text, base ?? null);
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-n5aw — the framework-agnostic SPARQL EDITOR core (highlighting + prefixes).
// Re-exported so the site (and the Tauri webview) consume the same dependency-free tokenizer
// and common-prefix logic. The React overlay editor lives in the site component tree; this is
// the part that "transfers" to any GUI framework unchanged.
// ---------------------------------------------------------------------------

export {
  type SparqlToken,
  type SparqlTokenType,
  tokenizeSparql,
} from "./sparql-highlight.js";

// [OPUS-4.8] sq-8uew — the Turtle/TriG/N-Triples/N-Quads highlighting tokenizer, the sibling
// of `tokenizeSparql`. Shares the `SparqlTokenType` token classes (so one CSS palette styles
// both) and powers the RDF-document highlighting in the playground results / dataset views.
export {
  type TurtleToken,
  type TurtleTokenType,
  tokenizeTurtle,
} from "./turtle-highlight.js";

// [OPUS-4.8] sq-ixc3.1 — the JSON-LD highlighting tokenizer, the third sibling of
// `tokenizeSparql`/`tokenizeTurtle`, completing the data-formats picker's set. JSON-LD is JSON,
// so this is a small forgiving JSON lexer (keys / string values / numbers / `true`/`false`/
// `null` / `@…` JSON-LD keywords / punctuation). Shares the `SparqlTokenType` classes (so one
// CSS palette styles all three) and powers the JSON-LD editor/display on the site.
export {
  type JsonLdToken,
  type JsonLdTokenType,
  tokenizeJsonLd,
} from "./jsonld-highlight.js";

// [OPUS-4.8] sq-gb4o (#805) — the pretty/indented Turtle serialiser. The engine answers
// CONSTRUCT/DESCRIBE (and the dataset viewer's all-quads query) as FLAT N-Triples; this reshapes
// that into idiomatic grouped Turtle (`@prefix` abbreviation, subject blocks, `;`/`,` lists)
// while staying round-trip-equivalent. Dependency-free + DOM-free so the site and the Tauri
// webview share it. Compose with the highlighter as pretty-print THEN `tokenizeTurtle`.
export {
  type PrettyTurtleOptions,
  type RdfStatement,
  type RdfTerm,
  parseNTriples,
  prettyTrig,
  prettyTurtle,
} from "./pretty-turtle.js";

export {
  type PrefixBinding,
  COMMON_PREFIXES,
  declaredPrefixes,
  declaredPrefixBindings,
  usedPrefixes,
  missingCommonPrefixes,
  renderPrefixLines,
  withPrefixes,
} from "./sparql-prefixes.js";

// [OPUS-4.8] sq-x0kp — the framework-agnostic SPARQL-RESULTS formatting core (table
// extraction, CSV/TSV export, raw-JSON pretty-print, ASK discrimination). Pure data shaping
// over the SPARQL-JSON shapes above, re-exported so the site results panel AND the Tauri
// webview render the same cells from one source.
export {
  type ResultTable,
  resultVars,
  resultRows,
  isAskResult,
  askValue,
  extractTable,
  csvCell,
  tsvCell,
  resultsToCsv,
  resultsToTsv,
  formatSparqlJson,
} from "./results.js";

// [OPUS-4.8] sq-9w4t (#817) — paginated result rendering + a bounded read-ahead page cache:
// slice ONE display-page of a kept SELECT result, warming ~2 pages ahead so the next page is
// instant, while holding at most a bounded window of shaped slices. Pure shaping over the rows
// already streamed into JS (the engine result path is still materialised — the demand-driven
// EVALUATION half is gated on a pull/iterator exec model; see results.ts + gui-design.md §A.4).
export {
  type ResultPage,
  type PageCache,
  pageCount,
  clampPage,
  paginateTable,
  createPageCache,
} from "./results.js";

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-2mke — endpoint mode: the SPARQL 1.1 Protocol HTTP client.
// The companion to the in-tab WASM `Store` above — run the SAME editor against any
// running SPARQL 1.1 Protocol endpoint over `fetch`, with bearer-auth and the honest
// connection-safety classifier. Consumes the existing `sparq-server` API; bypasses no
// server gate; claims no security the server does not provide.
// ---------------------------------------------------------------------------

export {
  type EndpointConfig,
  type EndpointForm,
  type EndpointResult,
  type PreparedRequest,
  type SafetyCode,
  type SafetyLevel,
  type SafetyWarning,
  EndpointError,
  SPARQL_GRAPH_NTRIPLES,
  SPARQL_RESULTS_JSON,
  buildSparqlRequest,
  classifyEndpointForm,
  connectionSafetyWarnings,
  hasBlockingWarning,
  isLoopbackHost,
  parseEndpointUrl,
  runEndpointQuery,
  wsSubprotocols,
} from "./endpoint.js";

// ---------------------------------------------------------------------------
// [FABLE-5] sq-ixc3.19 — the STRUCTURED query-plan client: the typed EXPLAIN tree
// (`explain_json::PlanNode`, the sq-jbqh4 camelCase schema contract) shared by all
// three plan sources (in-tab wasm `explainPlanJson`, desktop-native `explain_native`,
// server `Accept: application/x-sparq-explain+json`), the defensive parse, and the
// endpoint fetch with its honest structured → text degradation ladder.
// ---------------------------------------------------------------------------

export {
  type PlanNode,
  type PlanExplainMode,
  type EndpointExplainResult,
  EXPLAIN_JSON_CT,
  EXPLAIN_TEXT_CT,
  buildExplainRequest,
  parsePlanJson,
  parsePlanNode,
  runEndpointExplain,
} from "./plan.js";

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-9ij6 — live subscriptions: the SSE transport of the server's
// `/subscriptions` surface. Subscribe a SELECT and stream the result DELTAS as the dataset
// mutates, reusing the SAME `EndpointConfig` + bearer posture as the endpoint query client
// (the token is sent only in the `Authorization: Bearer` header the server's SSE read gate
// validates). Consumes the existing `sparq-server` subscriptions API; bypasses no gate.
// ---------------------------------------------------------------------------

export {
  type SubscribedAck,
  type SubscriptionNotification,
  type SubscriptionError,
  type SubscriptionEvent,
  type SubscriptionHandlers,
  type SubscriptionHandle,
  type LiveResultSet,
  applyNotification,
  buildSubscriptionUrl,
  emptyLiveResultSet,
  frameData,
  liveResults,
  openSubscription,
  parseSubscriptionData,
  rowKey,
  splitSseFrames,
} from "./subscriptions.js";

// ---------------------------------------------------------------------------
// [FABLE-5] sq-140b — live subscriptions, the MULTIPLEXED WebSocket transport of the same
// `/subscriptions` surface: ONE socket carries many subscriptions via the server's
// `subscribe` / `unsubscribe` frames, with the bearer token offered only as the
// `bearer.<token>` subprotocol (`wsSubprotocols`) the server's `ws_auth_gate` validates.
// Speaks the SAME SEPA envelopes as the SSE transport (`parseSubscriptionData` and the
// `applyNotification` reducers are reused verbatim); bypasses no server gate.
// ---------------------------------------------------------------------------

export {
  type SubscriptionSocket,
  type SubscriptionSocketHandlers,
  type WebSocketCtor,
  type WebSocketLike,
  buildSubscriptionSocketUrl,
  openSubscriptionSocket,
} from "./ws-subscriptions.js";

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-he72 — server health / capabilities: read the connected server's
// operational surface (`/health`, the Prometheus `/metrics`, and the opt-in VoID / SPARQL
// Service Description). Reuses the SAME `EndpointConfig` + bearer posture as the query client;
// parses the Prometheus text exposition + the RDF descriptors into readable structures, and
// reports the server's honest `404` for a disabled opt-in feature as a `"not-exposed"`
// outcome (never a fabricated value). Consumes the existing `sparq-server` API; bypasses no
// gate; claims no capability the server does not advertise.
// ---------------------------------------------------------------------------

export {
  type CapabilitiesSummary,
  type FetchOutcome,
  type MetricFamily,
  type MetricSample,
  type MetricType,
  type NamedGraphSummary,
  type ParsedMetrics,
  type ServerHealth,
  type ServiceDescriptionSummary,
  type VoidSummary,
  HEALTH_PATH,
  METRICS_PATH,
  VOID_PATH,
  deriveServerUrl,
  extractCapabilities,
  extractServiceDescription,
  extractVoidSummary,
  fetchServerHealth,
  formatMetricLabels,
  parsePrometheusMetrics,
  shortenIri,
} from "./server-health.js";

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-atb0 — the persistent cross-session WORKSPACE model + persistence
// abstraction. The framework-agnostic data shape of a named workspace (its imported sources,
// the whole-dataset N-Quads save/open snapshot, and the saved SPARQL editor state) plus the
// ONE `WorkspaceStore` interface with three runtime-selected backends — Tauri on-disk (when a
// Tauri webview exposes the fs plugin), browser `localStorage` (the static-export path), and
// an in-memory session fallback. No Tauri/DOM dependency: each tier is feature-detected. No
// secret (a bearer token) is ever persisted. Consumed by the site's REPL workspace panel and
// portable to the Tauri GUI unchanged.
// ---------------------------------------------------------------------------

export {
  type Workspace,
  type WorkspaceSummary,
  type WorkspaceSourceMeta,
  type WorkspaceEditorState,
  type WorkspaceRunMode,
  type WorkspaceInferenceMode,
  // [sq-glo5r] — N3 per-workspace rule documents.
  type WorkspaceRulesDoc,
  type WorkspaceBackend,
  type WorkspaceStore,
  type WorkspaceStoreOptions,
  type KeyValueStorage,
  type TauriFsApi,
  // (sq-9i1h9) serialized per-workspace saves + (sq-w3dmj) save-failure surfacing.
  type SerializedWorkspaceSaver,
  createSerializedWorkspaceSaver,
  describeWorkspaceSaveError,
  isQuotaExceededError,
  WORKSPACE_SCHEMA,
  WORKSPACE_INFERENCE_MODES,
  parseInferenceMode,
  // [FABLE-5] sq-ixc3.14 — per-workspace federation egress allowlist (fail-closed parse).
  parseServiceAllowlist,
  WebWorkspaceStore,
  MemoryWorkspaceStore,
  TauriWorkspaceStore,
  createWorkspaceStore,
  detectWebStorage,
  isTauriRuntime,
  newWorkspace,
  parseWorkspace,
  workspaceId,
  workspaceSummary,
} from "./workspace.js";

// ---------------------------------------------------------------------------
// [GPT-5.6] sq-epbw4 — shared browser dataset decompression. gzip/ZIP use native
// browser streams; zstd/bzip2 stay in independent lazy chunks loaded only on demand.
// ---------------------------------------------------------------------------

export {
  type DatasetCompressionCodec,
  type DecompressedDatasetBytes,
  decompressDatasetBytes,
} from "./decompress.js";

/** Renders a term for display, with a compact datatype/lang suffix. */
export function formatTerm(t: SparqlTerm | undefined): string {
  if (!t) return "";
  if (t.type === "uri") return `<${t.value}>`;
  if (t.type === "bnode") return `_:${t.value}`;
  if (t["xml:lang"]) return `"${t.value}"@${t["xml:lang"]}`;
  if (t.datatype && t.datatype !== "http://www.w3.org/2001/XMLSchema#string") {
    const short = t.datatype.replace("http://www.w3.org/2001/XMLSchema#", "xsd:");
    return `"${t.value}"^^${short}`;
  }
  return `"${t.value}"`;
}
