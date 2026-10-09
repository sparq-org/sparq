"use client";

// [OPUS-4.8] sq-ixc3.9 — the operational engine context: the ONE live wasm store the whole
// workbench shares, plus warm status, the measured-latency query path, and the dataset
// summary the left-rail datasets tree renders.
//
// HONESTY: no performance number is baked in. The bottom status bar shows the latency of the
// query the user JUST ran, measured with `performance.now()` and labelled as such — never a
// benchmark claim. On the desktop Tauri target the design's end state is the DIRECT native
// engine link (gui/src-tauri/src/engine.rs); this foundation shell runs the same in-tab WASM
// engine in both targets (the honest, working-today path) and the IPC swap is a later phase.

import * as React from "react";
import {
  loadSparq,
  prewarmSparq,
  formatSparqlJson,
  isAskResult,
  askValue,
  streamQueryRows,
  COMMON_PREFIXES,
  termValue,
  type SparqlBinding,
  type SparqlResults,
  type WasmStore,
  type WasmStoreCtor,
  type WorkspaceInferenceMode,
  type WorkspaceRulesDoc,
  // [FABLE-5] sq-ixc3.19 — the typed EXPLAIN tree (the sq-jbqh4 camelCase schema
  // contract) + its defensive parse; shared by the wasm / native / endpoint plan sources.
  type PlanNode,
  parsePlanJson,
} from "@sparq/client";

import { basePath } from "@/lib/base-path";
import { SAMPLE_TURTLE, SAMPLE_FORMAT } from "@/data/sample-graph";
import { type ExportFormat } from "@/lib/rdf-format";
import {
  hasNativeFederation,
  hasNativeLoader,
  nativeDiskUsage,
  nativeExplain,
  nativeLoadPath,
  nativeLoadText,
  nativeServiceQuery,
  type LoadedDocument,
} from "@/lib/tauri-ipc";
// [FABLE-5] sq-ixc3.14 — federated SERVICE routing: the pure detector + native-result parse +
// fail-closed refusal classifier (see lib/federation.ts for the design note).
import {
  describeServiceRefusal,
  parseServiceResults,
  queryUsesService,
  WEB_FEDERATION_MESSAGE,
} from "@/lib/federation";
// [OPUS-4.8] sq-tp1m (#757) — the tier-b W-reason wasm bundle loader: the REAL forward-chaining
// RDFS / OWL 2 RL reasoner (crates/sparq-reason via sparq-reason-wasm), lazy-loaded so the lean
// query engine pays nothing until a workspace turns inference on.
import { loadReasoner, modeToProfile, type WasmReasoner } from "@/lib/reason-wasm";
import { classifyQuery } from "@/lib/query-form";
// [FABLE-5] sq-ixc3.20 — canonical triple identity for the inferred-fact affordance (the
// entailed fact cache the results views consult) + the shared N-Triples term writer (moved out
// of this file so the click-to-explain path and the snapshot writer share ONE writer).
import {
  entailedFactsFromClosure,
  termToNT,
  tripleKeysOfNTriples,
  type InferredFact,
} from "@/lib/inferred-facts";

/**
 * Internal sentinel thrown out of the streaming loop when the caller's {@link AbortSignal} fires,
 * so a Stop is distinguishable from a real engine error in the `catch` below. Named so the
 * instanceof check is robust to minification.
 */
class AbortError extends Error {
  constructor() {
    super("aborted");
    this.name = "AbortError";
  }
}

/** The engine warm lifecycle. `error` carries a load/parse failure message. */
export type EngineStatus =
  | { kind: "cold" }
  | { kind: "warming" }
  | { kind: "ready" }
  | { kind: "error"; message: string };

/** A per-named-graph row for the datasets tree (default graph + named graphs). */
export interface GraphSummary {
  /** The graph IRI, or null for the default graph. */
  graph: string | null;
  /** Triple/quad count in this graph. */
  count: number;
}

/** [GPT-5.6] sq-3eukz — one subject resource selectable as a Forms focus node. */
export interface FormResource {
  kind: "iri" | "bnode";
  value: string;
}

/**
 * The SELECT result the workbench renders. A streamed SELECT keeps at most {@link RunOptions.rowCap}
 * rows in JS (so a large result cannot blow the tab's memory); `truncated` records whether the
 * full result exceeded that cap. `rawJson` is a SPARQL-1.1-JSON document over the KEPT rows.
 */
export interface SelectOutcome {
  kind: "select";
  /** The SPARQL-JSON results doc over the kept rows (for the Table / Raw JSON views + export). */
  results: SparqlResults;
  rawJson: string;
  /** Rows kept in JS (≤ rowCap). */
  rowCount: number;
  /** Total rows the engine produced (may exceed `rowCount` when streamed + capped). */
  totalRows: number;
  /** True when the engine produced more rows than were kept in JS. */
  truncated: boolean;
}

/** What a run produced — discriminated by SPARQL form so the results panel can branch. */
export type QueryOutcome =
  | SelectOutcome
  | { kind: "ask"; value: boolean; rawJson: string }
  | { kind: "graph"; ntriples: string; tripleCount: number }
  | { kind: "update"; sizeAfter: number }
  // [FABLE-5] sq-ixc3.19 — `tree` is the STRUCTURED plan (the sq-jbqh4 schema contract)
  // when the source can produce one; `plan` keeps the text form (the lean-bundle
  // fallback — exactly one of the two is populated, never a fabricated tree from text).
  // `source` drives the wall-time honesty note: the in-tab wasm engine reads 0 nanos
  // (no monotonic clock), only the desktop-native / endpoint sources measure real time.
  | {
      kind: "explain";
      mode: "explain" | "analyze";
      plan: string;
      tree?: PlanNode;
      source?: "wasm" | "native";
    }
  | { kind: "cancelled" }
  | { kind: "error"; message: string };

/** A completed run + its MEASURED latency (performance.now delta, ms). */
export interface RunResult {
  outcome: QueryOutcome;
  /** Wall-clock latency of THIS run, measured with performance.now() (ms). Labelled, not a benchmark. */
  latencyMs: number;
}

/** How to run a query — plain execution, EXPLAIN (plan only), or EXPLAIN ANALYZE (plan + run). */
export type RunMode = "run" | "explain" | "analyze";

/**
 * [OPUS-4.8] sq-tp1m (#757) — counts describing what the active inference regime ADDED over the
 * live store. `baseTriples` is the distinct asserted triples the reasoner saw (named graphs are
 * folded into the default graph by the forward-chainer); `closureTriples` the size of the
 * materialised closure (base + entailed); `entailed` the delta reasoning produced. Real measured
 * counts from the engine's reasoner + store, never fabricated.
 */
export interface ReasoningInfo {
  mode: Exclude<WorkspaceInferenceMode, "off">;
  baseTriples: number;
  closureTriples: number;
  entailed: number;
}

/**
 * [OPUS-4.8] sq-tp1m — the reasoning lifecycle for the active workspace's inference mode:
 *   * `off`     — no inference regime (queries run over the asserted store);
 *   * `loading` — the W-reason bundle is being fetched / the closure materialised;
 *   * `ready`   — the closure is materialised; queries run against it ({@link ReasoningInfo});
 *   * `error`   — the reasoning bundle failed to load/run (e.g. it was not synced into this
 *                 build) — surfaced honestly; queries then fail with a clear message until the
 *                 user turns inference off or the bundle is rebuilt.
 */
export type InferenceStatus =
  | { kind: "off" }
  | { kind: "loading" }
  | { kind: "ready"; info: ReasoningInfo }
  | { kind: "error"; message: string };

/** Optional per-run controls. */
export interface RunOptions {
  /** plain run / EXPLAIN / EXPLAIN ANALYZE. Defaults to `"run"`. */
  mode?: RunMode;
  /**
   * Max SELECT rows to keep in JS when streaming (the cap that bounds peak memory). Rows beyond
   * this are counted but dropped; the outcome is marked `truncated`. Defaults to {@link DEFAULT_ROW_CAP}.
   */
  rowCap?: number;
  /** A cooperative cancel signal — checked between streamed batches (the Stop button). */
  signal?: AbortSignal;
  /**
   * [FABLE-5] sq-ixc3.19 — with mode `"explain"`/`"analyze"`, run the explain over the
   * DESKTOP-NATIVE engine (the `explain_native` Tauri command) instead of the in-tab wasm
   * engine: the only source that measures REAL per-operator wall nanos (wasm reads 0 — no
   * monotonic clock). Snapshot semantics match federation (`query_service`): the whole
   * (possibly reasoned) target store crosses as N-Quads. Web builds degrade to a clear
   * error, never a silent wasm fallback mislabelled as native. Ignored for mode `"run"`.
   */
  native?: boolean;
}

/**
 * The default cap on SELECT rows kept in JS for the table/JSON views (streaming bounds peak
 * memory at one batch + this many displayed rows; exports re-stream the WHOLE result). This is a
 * UI display bound, not a result bound — it is labelled in the results panel, not a benchmark.
 */
export const DEFAULT_ROW_CAP = 5_000;

/** The batch size {@link streamQueryRows} pulls per cursor step (one batch held at a time). */
const STREAM_BATCH_SIZE = 1_000;

/**
 * [FABLE-5] sq-ixc3.19 — the OPT-IN structured-explain bindings (the wasm crate's
 * `explain-json` feature, enabled in the published bundle). Feature-detected at runtime so a
 * lean bundle (which lacks the methods) degrades to the TEXT plan — never a fabricated tree.
 */
interface PlanCapableStore {
  explainPlanJson(sparql: string): string;
  explainPlanAnalyzeJson(sparql: string): string;
}

/** Where an import came from — drives the source kind recorded in the workspace metadata. */
export type ImportKind = "file" | "url" | "paste";

/** Whether an import REPLACES the live store or ADDS (merges) into it. */
export type ImportMode = "replace" | "add";

/** A request to bring RDF into the live store via the Import drawer. */
export interface ImportRequest {
  kind: ImportKind;
  /** REPLACE the store, or merge into it. */
  mode: ImportMode;
  /** Preserve named graphs (route quad-bearing formats through the dataset loader). */
  preserveGraphs: boolean;
  /** A human label for the source (filename / URL tail / "pasted document"). */
  label: string;
  /** The RDF serialisation to parse the document as (e.g. `turtle` / `nquads` / `hdt`). */
  format: string;
  /**
   * For `kind: "file"` — the disk path (the native loader reads + decodes it, incl. compressed
   * + native-only HDT). Mutually exclusive with `text`.
   */
  path?: string;
  /**
   * For `kind: "paste"` / `"url"` — the document body. Mutually exclusive with `path`. (The URL
   * fetch happens in the drawer; the fetched body is passed here.)
   */
  text?: string;
  /** For `kind: "url"` — the source URL, recorded so the workspace source is re-fetchable. */
  url?: string;
}

/** The outcome of a successful import — what the drawer reports + records as workspace metadata. */
export interface ImportResult {
  /** Quads ADDED by this import (the parsed document's whole-dataset count). */
  added: number;
  /** Total quads in the live store after the import. */
  storeSize: number;
  /** Whether the native (Tauri) loader handled it, or the in-tab WASM fallback did. */
  loadedNatively: boolean;
  /** The format the document was actually parsed as. */
  format: string;
  /** UTF-8 byte length of the imported document body (best-effort; for an "≈N bytes" note). */
  bytes: number;
}

/**
 * [OPUS-4.8] sq-vw3ax (#820) — the live INGEST signal the status bar's unintrusive ingest meter
 * reads. While an import is decoding/merging, `active` is true and `label` names the source; the
 * elapsed time is MEASURED by the status bar from `startedAt` (performance.now). The native + WASM
 * loaders are SYNCHRONOUS (no byte-level progress callback exists), so this is honestly an
 * indeterminate "ingesting <file>…" with a real elapsed — never a fabricated percentage or ETA.
 * `null` when no import is in flight.
 */
export interface IngestState {
  /** A human label for the source being ingested (filename / URL tail / "pasted document"). */
  label: string;
  /** performance.now() at ingest start — the status bar derives the live elapsed from this. */
  startedAt: number;
}

/**
 * [OPUS-4.8] sq-vw3ax (#820) — the live on-device store FOOTPRINT the status bar's disk gauge
 * reads. This is the REAL byte length of the whole-dataset N-Quads snapshot the workspace persists
 * (the `dataSnapshot` save/open cache, sq-atb0) — i.e. the actual size of what is written to disk
 * on the desktop target (or to localStorage on the web). It is a real measured figure, not a
 * fabricated capacity: there is no fixed cap, so the gauge shows the snapshot bytes, recomputed
 * after every load/import/update. A precise filesystem probe of the app-data dir (vs this snapshot
 * estimate) is a follow-up that needs a new Tauri command + capability (beaded).
 */
export interface EngineContextValue {
  status: EngineStatus;
  /** Total quads in the live store (default + all named graphs). */
  storeSize: number;
  /**
   * [OPUS-4.8] sq-vw3ax (#820) — the on-device store footprint ESTIMATE in bytes (the UTF-8 length
   * of the persisted whole-dataset N-Quads snapshot). 0 before the store warms. Honest measured
   * value, but an estimate of the on-disk size — it omits the workspace index JSON + encoding
   * overhead. On the desktop shell {@link diskBytes} reports the OS's exact figure instead.
   */
  storeBytes: number;
  /**
   * [OPUS-4.8] sq-cno90 (#820 follow-up) — the OS-REPORTED on-disk byte total of the
   * `$APPLOCALDATA/workspaces` tree (a recursive native `stat()` sum via the `disk_usage` command),
   * or `null` on the web target / before the first probe / if the probe failed. When present this
   * is the precise on-disk footprint the status bar PREFERS over the {@link storeBytes} estimate;
   * when `null` the gauge falls back to the estimate, labelled as such. Never fabricated.
   */
  diskBytes: number | null;
  /**
   * [OPUS-4.8] sq-cno90 (#820 follow-up) — re-run the OS-reported `disk_usage` probe (e.g. after a
   * workspace SAVE, which an import triggers, so {@link diskBytes} reflects the just-written file).
   * A no-op on the web target. Best-effort: a failure leaves the last value, never a fabrication.
   */
  refreshDiskUsage: () => void;
  /** [OPUS-4.8] sq-vw3ax (#820) — the in-flight import, or null. Drives the status bar ingest meter. */
  ingest: IngestState | null;
  /** Per-graph counts for the datasets tree. */
  graphs: GraphSummary[];
  /** The latency (ms) of the most recent run, or null before any run. */
  lastLatencyMs: number | null;
  /** The row count of the most recent SELECT run, or null. */
  lastRowCount: number | null;
  /**
   * Run a query/update against the live store; resolves with the outcome + measured latency.
   * [OPUS-4.8] sq-ixc3.10/.12 — this is the SINGLE EXPLAIN entry point: pass
   * {@link RunOptions.mode} `"explain"` / `"analyze"` to render the planner's plan (the canonical
   * EXPLAIN path the Cmd-K spine AND the workbench EXPLAIN/ANALYZE buttons both drive — there is
   * no separate `explain()` method). Pass {@link RunOptions.signal} to make the run cancellable
   * (Stop), and {@link RunOptions.rowCap} to bound the kept SELECT rows.
   */
  run: (query: string, opts?: RunOptions) => Promise<RunResult>;
  /**
   * [OPUS-4.8] sq-ixc3.11 — serialise the LIVE store to TriG so an operational tool (e.g. the
   * SHACL validator) can run over the actual imported store rather than a fixture. TriG (not
   * N-Triples — the serialise binding does not emit N-Triples) preserves every named graph as
   * well as the default graph. Returns `null` before the engine is ready or if the loaded
   * bundle lacks the serialise binding. The serialise-rdf binding is in the GUI's wasm bundle.
   */
  serializeStore: () => string | null;
  /**
   * [OPUS-4.8] sq-xvj9 — export the WHOLE live store to a pretty, human-readable RDF document
   * for download (the DatasetViewer's "Export data" action). Unlike {@link serializeStore} (an
   * internal, unabbreviated TriG dump the SHACL tool consumes), this is the USER-facing export:
   * indented + sorted, with IRIs abbreviated against the site's `COMMON_PREFIXES` (sq-l5kr's
   * caller-supplied prefix map), so `ex:`/`foaf:`/`schema:`/… compact exactly like the rest of
   * the workbench. `"turtle"` emits the DEFAULT GRAPH (Turtle has no named-graph syntax);
   * `"trig"` and `"jsonld"` emit the WHOLE dataset (default + every named graph). Returns `null`
   * before the engine is ready or if the loaded bundle lacks the serialise binding.
   */
  exportStore: (format: ExportFormat) => string | null;
  /**
   * [OPUS-4.8] sq-ixc3.13 — bring RDF into the live store via the Import drawer. A `file` import
   * (a disk path) goes through the NATIVE loader (`load_path`: threads, no wasm-tab ceiling,
   * compressed streams, native-only HDT) when running inside the Tauri desktop shell; `paste` /
   * `url` documents go through the native `load_text` there too, so named graphs are preserved by
   * the SAME engine path. On the hosted web target (no native loader) `paste` / `url` parse in
   * the in-tab WASM engine (no disk / compressed-file / HDT path — the drawer says so honestly).
   * `mode: "add"` MERGES into the current store (named-graph-preserving) instead of replacing it.
   * Resolves with the import outcome; rejects with the loader's error message on a parse failure.
   */
  importRdf: (req: ImportRequest) => Promise<ImportResult>;
  /** True when the native loader IPC is available (inside the Tauri desktop shell). */
  nativeLoaderAvailable: boolean;
  /**
   * [GPT-5.6] sq-3eukz — enumerate focus-node candidates from the active live store without
   * routing an internal picker refresh through the user-visible query-run metrics.
   */
  listFormResources: () => FormResource[];
  /**
   * [GPT-5.6] sq-3eukz — invoke the optional sparq-wasm `Store.deriveForm` method against the
   * live runtime. Returns `null` when that feature-detected method is absent; otherwise returns
   * the host's snake_case FormDescription JSON (or throws its derivation error).
   */
  wasmDeriveForm: (
    data: string,
    shapes: string,
    focus: string,
    format: string,
    optionsJson: string,
  ) => string | null;
  /**
   * [OPUS-4.8] sq-tp1m (#757) — the active per-workspace INFERENCE regime (query-time entailment).
   * `"off"` runs plain SPARQL; `"rdfs"` / `"owl-rl"` forward-chain the deductive closure (via the
   * real `sparq-reason` W-reason bundle) so a query matches entailed triples too. This is the
   * ENGINE's view of the mode; PERSISTING the choice per workspace is the workspace context's job
   * (the two are kept in lockstep by the inference-mode bridge).
   */
  inferenceMode: WorkspaceInferenceMode;
  /** [OPUS-4.8] sq-tp1m — the reasoning bundle/closure lifecycle for {@link inferenceMode}. */
  inferenceStatus: InferenceStatus;
  /**
   * [OPUS-4.8] sq-tp1m — set the active inference regime. `"off"` tears down the closure and runs
   * plain SPARQL; `"rdfs"` / `"owl-rl"` lazily load the W-reason bundle and materialise the
   * forward-chaining closure over the live store so subsequent queries see entailed triples. The
   * closure is a QUERY-TIME view — it never mutates the persisted store — and is rebuilt
   * automatically when the store changes (import / update).
   */
  setInferenceMode: (mode: WorkspaceInferenceMode) => void;
  /**
   * [FABLE-5] sq-ixc3.20 — the canonical triple keys (see @/lib/inferred-facts) of every
   * ENTAILED triple the ACTIVE closure added over the asserted base, or `null` when no
   * closure is materialised (inference off / still loading / errored). This is what lets the
   * results views mark an inferred row/edge and offer the why() affordance — membership is
   * exact set difference (closure − base), never a heuristic, so an asserted fact can never
   * carry the "inferred" affordance. Recomputed with the closure (same cache discipline).
   */
  entailedTripleKeys: () => ReadonlySet<string> | null;
  /**
   * [GPT-5.6] The same exact closure-added set as {@link entailedTripleKeys}, retained in the
   * N-Triples term spelling accepted by {@link explainWhy}. Used by the Inference tool's fact
   * browser; guarded by the same active closure cache key, so stale facts are never exposed.
   */
  entailedTripleFacts: () => readonly InferredFact[] | null;
  /**
   * [FABLE-5] sq-ixc3.20 — ONE derivation ("why?") of the triple `(s, p, o)` under the
   * ACTIVE inference regime, as `sparq-reason`'s proof-tree JSON (parse with
   * @/lib/proof-view `parseProofJson`; the string `"null"` means "not entailed"). `s`/`p`/`o`
   * are N-Triples term strings (the @/lib/inferred-facts `termToNT` form). Runs the W-reason
   * bundle's `why` (RDFS / OWL-RL) or `whyN3` (N3 rules) over the asserted base — a witness,
   * not an enumeration of all derivations. Rejects honestly when inference is off, the
   * engine is cold, or the synced bundle predates the explain surface.
   */
  explainWhy: (s: string, p: string, o: string) => Promise<string>;
  /**
   * [sq-glo5r] Push the active workspace's N3 rules docs to the engine so the N3 closure is
   * rebuilt when rules change. Called by `N3RulesBridge` (on workspace rule mutations) and
   * directly by workspace lifecycle actions (workspace switch, initial restore) as belt-and-
   * suspenders in lockstep with `setInferenceMode`. A no-op when inference is not `"n3"` — the
   * rules are cached by content hash so a no-change push is free.
   */
  setN3Rules: (docs: WorkspaceRulesDoc[]) => void;
  /**
   * [FABLE-5] sq-ixc3.14 — push the active workspace's FEDERATION egress allowlist to the
   * engine, so a SERVICE-bearing run hands the native `query_service` command exactly the
   * endpoints the user allowlisted for THIS workspace. Kept in lockstep with the persisted
   * workspace setting by `FederationBridge` (mirrors the inference-mode bridge). FAIL-CLOSED:
   * until pushed (or when empty), every SERVICE endpoint is refused by the native engine.
   */
  setServiceAllowlist: (hosts: string[]) => void;
  /**
   * [FABLE-5] sq-ixc3.14 — whether the native federated-query IPC is available (inside the
   * Tauri desktop shell). On the web build this is false and a SERVICE-bearing run degrades
   * honestly (native-only) instead of hanging on CORS.
   */
  nativeFederationAvailable: boolean;
  /**
   * [OPUS-4.8] sq-ixc3.13 — the whole-dataset N-QUADS snapshot of the live store (default graph +
   * every named graph), the save/open cache a workspace persists as its `dataSnapshot` (sq-atb0).
   * N-Quads (not the TriG `serializeStore` emits) because that is the workspace snapshot format,
   * and it agrees byte-for-byte with the "add to current" merge path. `null` before warm.
   */
  snapshotStore: () => string | null;
  /**
   * [FABLE-5] sq-ixc3.16 — the live store's CONTENT epoch: bumps whenever the base store's
   * content changes (hydration / import / a successful SPARQL UPDATE). Already the internal
   * signal the inference closure rebuilds on (sq-tp1m); exposed so an operational tool (the
   * Streaming tool's workspace feed) can react to workspace updates without polling. Strictly
   * monotone within a session; carries no content itself — pair with {@link snapshotStore}.
   */
  storeEpoch: number;
  /**
   * [OPUS-4.8] sq-lcd6e — REPLACE the live store with a restored workspace's persisted SNAPSHOT
   * (its whole-dataset N-Quads). This is the fix for the silent data-loss-on-relaunch: the warm
   * path no longer unconditionally seeds the sample graph — the store's INITIAL content comes
   * from the workspace being restored.
   *   * `nquads` non-empty → the store is rebuilt from it (the imported data survives a reload);
   *   * empty / `null` + `seedSampleWhenEmpty` → seed the SAMPLE graph (a genuinely-fresh
   *     first-run default workspace, so a new user has something to query);
   *   * empty / `null` without the flag → an EXPLICITLY-empty workspace stays empty.
   * Bumps the store epoch so an active inference closure rematerialises over the restored data
   * (sq-tp1m), refreshes the datasets summary + footprint, and marks the engine ready. If the
   * wasm ctor has not warmed yet the request is QUEUED and applied the moment warm-up finishes,
   * so a restore that races the cold-start is never dropped.
   */
  hydrateFromSnapshot: (
    nquads: string | null,
    opts?: { seedSampleWhenEmpty?: boolean },
  ) => void;
}

const EngineContext = React.createContext<EngineContextValue | null>(null);

// [OPUS-4.8] sq-ixc3.13 — the all-quads N-Quads (de)serialisation the Import drawer's MERGE
// path uses. The wasm `Store.serialize` binding does NOT emit N-Triples/N-Quads (only
// turtle/trig/jsonld — see `serializeStore` below), and the framework-agnostic `@sparq/client`
// deliberately leaves the dataset-format opinion (which formats carry named graphs) to the
// host. So this small, well-understood serialiser lives here: SELECT every quad over the live
// store and emit one N-Quads line per solution, exactly the shape the site's `storeToNQuads`
// (site/src/lib/repl-dataset.ts) produces. The merge then concatenates this with the incoming
// document's N-Quads and re-loads with `loadDataset(_, "nquads")`, so BOTH stores' named graphs
// survive the round-trip.

/** SELECT every quad (default graph as the unbound `?g`, plus every named graph). */
const ALL_QUADS_QUERY =
  "SELECT ?s ?p ?o ?g WHERE { { ?s ?p ?o } UNION { GRAPH ?g { ?s ?p ?o } } }";

// [FABLE-5] sq-ixc3.20 — `termToNT` (the canonical N-Triples/N-Quads TERM writer for
// SPARQL-JSON terms; full datatype IRI + escaped lexical form, unlike the display-only
// `formatTerm`) now lives in @/lib/inferred-facts, shared with the click-to-explain path.

/**
 * Serialise the WHOLE dataset of the live store (default graph + every named graph) to N-Quads,
 * one line per quad. Used as the LEFT side of the "add to current" merge. An empty store yields
 * `""`.
 */
function storeToNQuads(store: WasmStore): string {
  const json = store.query(ALL_QUADS_QUERY);
  const parsed = JSON.parse(json) as SparqlResults;
  const lines: string[] = [];
  for (const b of parsed.results?.bindings ?? []) {
    const s = b["s"];
    const p = b["p"];
    const o = b["o"];
    if (!s || !p || !o) continue;
    const g = b["g"];
    const tail = g ? ` ${termToNT(g)}` : "";
    lines.push(`${termToNT(s)} ${termToNT(p)} ${termToNT(o)}${tail} .`);
  }
  return lines.join("\n");
}

/** [sq-glo5r] FNV-1a hash for a string — non-crypto, fast; used for the N3 rules cache key. */
function fnv1aHash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h;
}

// [sq-glo5r] — SELECT DISTINCT to fold named graphs into the default graph and deduplicate
// s/p/o. Used as the base data input for N3 rule reasoning (N3 does not accept N-Quads; named
// graphs are folded exactly as the RDFS/OWL-RL reasoner folds them).
const ALL_TRIPLES_QUERY =
  "SELECT DISTINCT ?s ?p ?o WHERE { { ?s ?p ?o } UNION { GRAPH ?g { ?s ?p ?o } } }";

// [GPT-5.6] sq-3eukz — subjects in either the default or a named graph are the live workspace's
// focus-resource candidates. Literal objects are intentionally not candidates for SHACL focus.
const FORM_RESOURCES_QUERY =
  "SELECT DISTINCT ?resource WHERE { { ?resource ?p ?o } UNION { GRAPH ?g { ?resource ?p ?o } } } ORDER BY ?resource LIMIT 200";

/**
 * [sq-glo5r] Serialise the whole dataset as N-TRIPLES, folding named graphs into the default
 * graph and deduplicating s/p/o (via DISTINCT). Returns `""` for an empty store. Used as the
 * base for N3 rule reasoning — N3 is a superset of Turtle which accepts full-IRI N-Triple lines.
 */
function storeToNTriples(store: WasmStore): string {
  const json = store.query(ALL_TRIPLES_QUERY);
  const parsed = JSON.parse(json) as SparqlResults;
  const lines: string[] = [];
  for (const b of parsed.results?.bindings ?? []) {
    const s = b["s"];
    const p = b["p"];
    const o = b["o"];
    if (!s || !p || !o) continue;
    lines.push(`${termToNT(s)} ${termToNT(p)} ${termToNT(o)} .`);
  }
  return lines.join("\n");
}

/** Summarise the live store into per-graph counts via a single grouped query. */
function summariseGraphs(store: WasmStore): { size: number; graphs: GraphSummary[] } {
  const graphs: GraphSummary[] = [];
  let size = 0;
  // Default graph.
  try {
    const def = store.count("SELECT * WHERE { ?s ?p ?o }");
    if (def > 0) {
      graphs.push({ graph: null, count: def });
      size += def;
    }
  } catch {
    /* count over an empty store can be zero; ignore */
  }
  // Named graphs (group by graph).
  try {
    const json = store.query(
      "SELECT ?g (COUNT(*) AS ?c) WHERE { GRAPH ?g { ?s ?p ?o } } GROUP BY ?g ORDER BY ?g",
    );
    const parsed = JSON.parse(json) as SparqlResults;
    for (const b of parsed.results?.bindings ?? []) {
      const g = termValue(b["g"]) ?? null;
      const c = Number.parseInt(termValue(b["c"]) ?? "0", 10) || 0;
      if (g) {
        graphs.push({ graph: g, count: c });
        size += c;
      }
    }
  } catch {
    /* a store with no named graphs yields no rows; ignore */
  }
  return { size, graphs };
}

/**
 * [OPUS-4.8] sq-vw3ax (#820) — the REAL on-device footprint of a store: the UTF-8 byte length of
 * its whole-dataset N-Quads snapshot (exactly what the workspace persists to disk / localStorage).
 * Used for the status bar disk gauge — a measured figure, never a fabricated capacity.
 */
function snapshotBytes(store: WasmStore): number {
  try {
    return new Blob([storeToNQuads(store)]).size;
  } catch {
    return 0;
  }
}

/**
 * [OPUS-4.8] sq-tp1m (#757) — measure what the active inference regime added: the distinct
 * asserted triples the reasoner saw (`base`, folded to the default graph exactly as the
 * forward-chainer folds named graphs), the closure size (`reasoned`, default-graph N-Triples),
 * and the entailed delta. All three are REAL counts queried from the two live stores — never a
 * fabricated figure. A count over an empty store can throw; treat that as zero.
 */
function computeReasoningInfo(
  base: WasmStore,
  reasoned: WasmStore,
  mode: Exclude<WorkspaceInferenceMode, "off">,
): ReasoningInfo {
  let baseTriples = 0;
  try {
    // Distinct s/p/o across the default graph AND every named graph — the reasoner folds named
    // graphs into the default graph, so this is the base-triple count it actually reasoned over.
    baseTriples = base.count(
      "SELECT DISTINCT ?s ?p ?o WHERE { { ?s ?p ?o } UNION { GRAPH ?g { ?s ?p ?o } } }",
    );
  } catch {
    /* empty store */
  }
  let closureTriples = 0;
  try {
    closureTriples = reasoned.count("SELECT * WHERE { ?s ?p ?o }");
  } catch {
    /* empty closure */
  }
  const entailed = Math.max(0, closureTriples - baseTriples);
  return { mode, baseTriples, closureTriples, entailed };
}

// [OPUS-4.8] sq-xvj9 — the caller-supplied prefix map (sq-l5kr) as the `[prefix, iri]` pairs the
// wasm `serialize` binding takes: the site's `COMMON_PREFIXES`, so an exported document abbreviates
// `ex:`/`foaf:`/`schema:`/… consistently with the rest of the workbench (and byte-parity with the
// site's serialiser opinion). Computed once — the registry is static.
const EXPORT_PREFIXES: [string, string][] = COMMON_PREFIXES.map((b) => [b.prefix, b.iri]);

// The engine `serialize` format string for each UI export format. JSON-LD uses the COMPACTED form
// so the prefix map drives a readable `@context` (the plain `"jsonld"` form is expanded and ignores
// `abbreviate`); Turtle/TriG abbreviate via a sorted `@prefix` header (the `abbreviate=true` flag).
const EXPORT_SERIALIZE_FORMAT: Record<ExportFormat, string> = {
  turtle: "turtle",
  trig: "trig",
  jsonld: "jsonld-compacted",
};

export function EngineProvider({ children }: { children: React.ReactNode }) {
  const [status, setStatus] = React.useState<EngineStatus>({ kind: "cold" });
  const [storeSize, setStoreSize] = React.useState(0);
  // [OPUS-4.8] sq-vw3ax (#820) — the live on-device footprint (snapshot bytes) + in-flight import.
  const [storeBytes, setStoreBytes] = React.useState(0);
  // [OPUS-4.8] sq-cno90 (#820 follow-up) — the OS-reported on-disk byte total of the workspaces
  // tree (desktop only); null on the web target, where the gauge uses the snapshot estimate.
  const [diskBytes, setDiskBytes] = React.useState<number | null>(null);
  const [ingest, setIngest] = React.useState<IngestState | null>(null);
  const [graphs, setGraphs] = React.useState<GraphSummary[]>([]);
  const [lastLatencyMs, setLastLatencyMs] = React.useState<number | null>(null);
  const [lastRowCount, setLastRowCount] = React.useState<number | null>(null);

  const storeRef = React.useRef<WasmStore | null>(null);
  const ctorRef = React.useRef<WasmStoreCtor | null>(null);

  // [FABLE-5] sq-ixc3.14 — the active workspace's FEDERATION egress allowlist, pushed by
  // `setServiceAllowlist` (the FederationBridge / workspace lifecycle). A ref (not state):
  // run() reads it imperatively at dispatch time and nothing re-renders on it. Starts EMPTY,
  // so until a workspace pushes its setting the native engine refuses every SERVICE endpoint
  // (fail-closed by construction).
  const serviceAllowRef = React.useRef<string[]>([]);

  // [OPUS-4.8] sq-tp1m (#757) — the per-workspace inference regime + its materialised closure.
  const [inferenceMode, setInferenceModeState] = React.useState<WorkspaceInferenceMode>("off");
  const [inferenceStatus, setInferenceStatus] = React.useState<InferenceStatus>({ kind: "off" });
  // The reasoned (closure) store read queries run against when a regime is active, plus the cache
  // key (`mode:epoch`) it was built for. `storeEpoch` bumps whenever the BASE store content
  // changes (warm / import / update) so a stale closure is rebuilt automatically.
  const reasonedStoreRef = React.useRef<WasmStore | null>(null);
  const reasonedKeyRef = React.useRef<string | null>(null);
  // [GPT-5.6] sq-l54uy — the canonical keys + displayable facts the ACTIVE closure ADDED
  // (closure − base), computed alongside the closure build and cached under the SAME
  // `mode:epoch[:rulesHash]` key so it can never describe a different store than
  // `reasonedStoreRef` (the guard in `entailedTripleKeys` checks the key, not just presence).
  const entailedCacheRef = React.useRef<{
    key: string;
    keys: Set<string>;
    facts: InferredFact[];
  } | null>(null);
  // [OPUS-4.8] sq-tp1m — the SINGLE-FLIGHT slot for an in-flight closure build, keyed by the
  // same `mode:epoch`. Overlapping callers (the applyInferenceMode effect + a run(), or two
  // run()s) that request the same key while the first materialisation is still awaiting share
  // this one Promise instead of each re-running the (expensive) materialize + racing to
  // overwrite reasonedStoreRef/reasonedKeyRef (mirrors modulePromise in reason-wasm.ts).
  const reasonedBuildRef = React.useRef<{ key: string; promise: Promise<WasmStore> } | null>(null);
  const storeEpochRef = React.useRef(0);
  const [storeEpoch, setStoreEpoch] = React.useState(0);
  // [OPUS-4.8] sq-tp1m — a generation counter bumped on every applyInferenceMode invocation. An
  // async materialisation that resolves AFTER a newer invocation (the user toggled modes quickly,
  // or the store epoch bumped mid-build) is stale and must NOT publish its status, or the UI could
  // land in the wrong state (e.g. mode Off but status Ready/Error). Only the latest generation writes.
  const inferenceGenRef = React.useRef(0);

  // [sq-glo5r] — the active workspace's N3 rule documents (pushed by setN3Rules / N3RulesBridge).
  // Held in a ref so buildReasonedStore always reads the latest docs without adding them to its
  // useCallback deps; `rulesEpoch` bumps on every setN3Rules call to trigger the applyInferenceMode
  // effect and rebuild the N3 closure when rules change while N3 is active.
  const n3RulesRef = React.useRef<WorkspaceRulesDoc[]>([]);
  const rulesEpochRef = React.useRef(0);
  const [rulesEpoch, setRulesEpoch] = React.useState(0);

  // [OPUS-4.8] sq-lcd6e — a hydration request that arrived BEFORE the wasm ctor warmed (the
  // workspace-restore bridge can call hydrateFromSnapshot while the engine is still cold). It is
  // applied by the warm effect the moment the ctor is ready, so a restore never races the
  // cold-start and gets dropped (which would silently fall back to an empty / sample store).
  const pendingHydrationRef = React.useRef<{
    nquads: string | null;
    seedSampleWhenEmpty: boolean;
  } | null>(null);

  // [OPUS-4.8] sq-lcd6e — (re)build the live store from a restored snapshot, then publish the
  // summary + ready status. The SINGLE place the store's content is (re)seeded, whether on the
  // initial warm (fresh default → sample) or a workspace switch (target's snapshot). Bumps the
  // store epoch + drops the cached closure so an active inference regime rematerialises over the
  // restored data (the sq-tp1m epoch discipline).
  const applyHydration = React.useCallback(
    (
      Store: WasmStoreCtor,
      req: { nquads: string | null; seedSampleWhenEmpty: boolean },
    ) => {
      const content = (req.nquads ?? "").trim();
      let store: WasmStore;
      try {
        if (content !== "") {
          // A restored workspace snapshot (whole dataset, N-Quads) — the imported data survives.
          store = Store.loadDataset(req.nquads as string, "nquads");
        } else if (req.seedSampleWhenEmpty) {
          // A genuinely-fresh first-run default workspace: seed the sample so there is something
          // to query. (An explicitly-empty workspace, below, stays empty.)
          store = Store.load(SAMPLE_TURTLE, SAMPLE_FORMAT);
        } else {
          // An explicitly-empty workspace: an empty store (an empty N-Quads document — the same
          // loader the merge path uses, so an empty parse is unambiguously zero quads).
          store = Store.loadDataset("", "nquads");
        }
      } catch (err: unknown) {
        setStatus({
          kind: "error",
          message: err instanceof Error ? err.message : String(err),
        });
        return;
      }
      storeRef.current = store;
      // The base store was replaced: drop any cached closure + in-flight build so an active
      // inference regime rematerialises over the RESTORED data rather than a stale prior store.
      reasonedStoreRef.current = null;
      reasonedKeyRef.current = null;
      reasonedBuildRef.current = null;
      entailedCacheRef.current = null;
      storeEpochRef.current += 1;
      setStoreEpoch(storeEpochRef.current);
      const { size, graphs: gs } = summariseGraphs(store);
      setStoreSize(size);
      setStoreBytes(snapshotBytes(store));
      setGraphs(gs);
      setStatus({ kind: "ready" });
    },
    [],
  );
  // Keep a stable ref so the warm effect (deps []) can apply a queued request without re-running.
  const applyHydrationRef = React.useRef(applyHydration);
  applyHydrationRef.current = applyHydration;

  const hydrateFromSnapshot = React.useCallback(
    (nquads: string | null, opts?: { seedSampleWhenEmpty?: boolean }) => {
      const req = { nquads, seedSampleWhenEmpty: opts?.seedSampleWhenEmpty ?? false };
      const Store = ctorRef.current;
      if (!Store) {
        // Cold engine: queue the request — the warm effect applies it once the ctor is ready.
        pendingHydrationRef.current = req;
        return;
      }
      applyHydration(Store, req);
    },
    [applyHydration],
  );

  // Warm the engine once on mount: load wasm, then HYDRATE the store from the restored workspace
  // (a queued request, or — if the restore bridge has not called yet — wait for it). The store's
  // initial content is NO LONGER an unconditional sample seed (sq-lcd6e: that silently replaced a
  // user's imported data on every relaunch); status stays `warming` until the first hydration.
  React.useEffect(() => {
    let cancelled = false;
    const opts = { basePath: basePath() };
    setStatus({ kind: "warming" });
    prewarmSparq(opts)
      .then(() => loadSparq(opts))
      .then((Store) => {
        if (cancelled) return;
        ctorRef.current = Store;
        const pending = pendingHydrationRef.current;
        if (pending) {
          pendingHydrationRef.current = null;
          applyHydrationRef.current(Store, pending);
        }
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setStatus({
          kind: "error",
          message: err instanceof Error ? err.message : String(err),
        });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // [OPUS-4.8] sq-cno90 (#820 follow-up) — probe the OS-reported on-disk byte total of the
  // workspaces tree via the native `disk_usage` command. On the desktop shell this is the precise
  // figure the status bar prefers; on the web target (no native FS) it stays `null` and the gauge
  // shows the snapshot estimate instead. Best-effort: a probe failure leaves the last value (or
  // `null`) — the gauge never fabricates a number. The on-disk figure lags a freshly persisted
  // snapshot by one write, so we re-probe after the workspace save that an import/update triggers.
  const refreshDiskUsage = React.useCallback(() => {
    nativeDiskUsage()
      .then((du) => {
        if (du) setDiskBytes(du.bytes);
      })
      .catch(() => {
        /* a probe failure must never break the status bar — keep the last value / estimate */
      });
  }, []);

  // [OPUS-4.8] sq-cno90 — probe the OS-reported on-disk footprint once on mount (desktop only; a
  // no-op that leaves diskBytes null on the web target). A restored workspace already has bytes on
  // disk, so this surfaces the real figure on first paint of the desktop shell.
  React.useEffect(() => {
    refreshDiskUsage();
  }, [refreshDiskUsage]);

  const refreshSummary = React.useCallback(() => {
    const store = storeRef.current;
    if (!store) return;
    const { size, graphs: gs } = summariseGraphs(store);
    setStoreSize(size);
    setStoreBytes(snapshotBytes(store));
    setGraphs(gs);
    refreshDiskUsage();
  }, [refreshDiskUsage]);

  // [OPUS-4.8] sq-tp1m (#757) — build (or reuse) the reasoned CLOSURE store for `mode` over the
  // current base store. For RDFS / OWL-RL: serialise to N-Quads + materialize with the W-reason
  // bundle. For N3 (sq-glo5r): fold named graphs to N-Triples, concatenate with enabled rules docs,
  // call reasonN3 for derived ground triples, load base + derived as the closure. Cached by a
  // `mode:epoch[:rulesHash]` key — the materialisation cost is paid once per change.
  // Throws if the bundle is unavailable or the closure fails — the caller surfaces that honestly.
  const buildReasonedStore = React.useCallback(
    async (mode: Exclude<WorkspaceInferenceMode, "off">): Promise<WasmStore> => {
      const Store = ctorRef.current;
      const base = storeRef.current;
      if (!Store || !base) {
        throw new Error("The engine is not ready yet — wait for the store to warm.");
      }

      // [sq-glo5r] — N3 rules mode: base is folded to N-Triples (same as RDFS/OWL-RL folding),
      // concatenated with enabled rules texts, then reasonN3 forward-chains the custom rules.
      // The cache key includes a content hash of the enabled rules so a rules change invalidates
      // the prior closure even if the store epoch is unchanged.
      if (mode === "n3") {
        const enabledRules = n3RulesRef.current.filter((d) => d.enabled !== false);
        const rulesText = enabledRules.map((d) => d.text).join("\n");
        const rulesHash = fnv1aHash(rulesText);
        const key = `n3:${storeEpochRef.current}:${rulesHash}`;
        if (reasonedKeyRef.current === key && reasonedStoreRef.current) {
          return reasonedStoreRef.current;
        }
        const n3Inflight = reasonedBuildRef.current;
        if (n3Inflight && n3Inflight.key === key) {
          return n3Inflight.promise;
        }
        const n3Promise = (async () => {
          const reasoner = await loadReasoner();
          const baseNTriples = storeToNTriples(base);
          // Combine rules (N3 Turtle) with base facts (N-Triple lines are valid N3/Turtle syntax).
          const combined = rulesText + "\n" + baseNTriples;
          // reasonN3 returns ONLY the newly entailed ground N-Triple lines (not the base triples).
          const derived = reasoner.reasonN3(combined);
          // Closure = base triples + derived (so queries over the closure see both).
          const closure = baseNTriples + (derived.trim() ? "\n" + derived : "");
          const reasoned = Store.load(closure || " ", "ntriples");
          // [FABLE-5] sq-ixc3.20 — the derived lines ARE the entailed set for N3 mode;
          // reduce them to canonical keys for the inferred-fact affordance.
          // [GPT-5.6] Retain the same exact derived set as explainable N-Triples facts for the
          // Inference tool browser. An empty base is correct because reasonN3 returns additions.
          const entailed = entailedFactsFromClosure(derived, new Set<string>());
          entailedCacheRef.current = { key, ...entailed };
          reasonedStoreRef.current = reasoned;
          reasonedKeyRef.current = key;
          return reasoned;
        })();
        reasonedBuildRef.current = { key, promise: n3Promise };
        try {
          return await n3Promise;
        } finally {
          if (reasonedBuildRef.current?.key === key) reasonedBuildRef.current = null;
        }
      }

      // RDFS / OWL-RL: N-Quads snapshot → materialize → N-Triples closure.
      // TypeScript narrows `mode` to `"rdfs" | "owl-rl"` here after the `=== "n3"` branch above.
      const key = `${mode}:${storeEpochRef.current}`;
      // Completed-closure cache hit: reuse the already-materialised store.
      if (reasonedKeyRef.current === key && reasonedStoreRef.current) {
        return reasonedStoreRef.current;
      }
      // Single-flight: a build for this exact key is already in flight — await THAT promise
      // rather than starting a second materialize() (the most expensive work) and racing to
      // overwrite the refs. Once it resolves the completed-closure cache above serves the rest.
      const inflight = reasonedBuildRef.current;
      if (inflight && inflight.key === key) {
        return inflight.promise;
      }
      const promise = (async () => {
        const reasoner = await loadReasoner();
        // The whole-dataset N-Quads snapshot; the reasoner folds named graphs into the default graph.
        const snapshot = storeToNQuads(base);
        const closure = reasoner.materialize(snapshot, "nquads", modeToProfile(mode));
        const reasoned = Store.load(closure, "ntriples");
        // [FABLE-5] sq-ixc3.20 — entailed = closure − base at CANONICAL key level (the two
        // texts come from different writers — Rust vs the JS termToNT — so identity is the
        // decoded key, never the raw line; see @/lib/inferred-facts).
        // [GPT-5.6] Compute closure-minus-base once, retaining both membership keys and the
        // exact N-Triples terms needed by the entailed-facts browser and why() panel.
        const entailed = entailedFactsFromClosure(closure, tripleKeysOfNTriples(snapshot));
        entailedCacheRef.current = { key, ...entailed };
        reasonedStoreRef.current = reasoned;
        reasonedKeyRef.current = key;
        return reasoned;
      })();
      reasonedBuildRef.current = { key, promise };
      try {
        return await promise;
      } finally {
        // Clear the in-flight slot only if it is still ours (a newer key may have replaced it),
        // so a failed build releases the slot and a later attempt can retry.
        if (reasonedBuildRef.current?.key === key) reasonedBuildRef.current = null;
      }
    },
    [],
  );

  // [OPUS-4.8] sq-tp1m — reconcile the reasoning STATUS with the active mode + current store:
  // tear the closure down for "off", else (re)materialise it and publish the entailed-triple
  // counts. Idempotent; the single place reasoning is (re)applied (see the effect below).
  const applyInferenceMode = React.useCallback(
    async (mode: WorkspaceInferenceMode): Promise<void> => {
      // Claim this generation up-front; any invocation started later supersedes us.
      const gen = ++inferenceGenRef.current;
      if (mode === "off") {
        reasonedStoreRef.current = null;
        reasonedKeyRef.current = null;
        entailedCacheRef.current = null;
        setInferenceStatus({ kind: "off" });
        return;
      }
      // The engine/store may not have warmed yet — e.g. a workspace REOPENED with inference
      // already on runs this before the mount warm-up seeds the store. That is a WARMING state,
      // not a reasoner failure: show `loading` and let the storeEpoch bump (fired when the store
      // warms) re-run this effect and retry. Reserve `error` for a genuine bundle-load /
      // materialisation failure, so the toolbar never flashes "reasoner unavailable" over a
      // reasoner that is in fact fine.
      if (!storeRef.current || !ctorRef.current) {
        setInferenceStatus({ kind: "loading" });
        return;
      }
      // Check if the closure for this mode + store epoch (+ rules hash for N3) is already built.
      const cachedKeyPrefix =
        mode === "n3"
          ? `n3:${storeEpochRef.current}:`
          : `${mode}:${storeEpochRef.current}`;
      const alreadyBuilt =
        reasonedKeyRef.current !== null &&
        reasonedStoreRef.current !== null &&
        (mode === "n3"
          ? reasonedKeyRef.current.startsWith(cachedKeyPrefix)
          : reasonedKeyRef.current === cachedKeyPrefix);
      if (!alreadyBuilt) setInferenceStatus({ kind: "loading" });
      try {
        const reasoned = await buildReasonedStore(mode);
        // A newer invocation ran while we awaited — drop this stale result so we don't clobber it.
        if (gen !== inferenceGenRef.current) return;
        const base = storeRef.current;
        const info: ReasoningInfo = base
          ? computeReasoningInfo(base, reasoned, mode)
          : { mode, baseTriples: 0, closureTriples: 0, entailed: 0 };
        setInferenceStatus({ kind: "ready", info });
      } catch (err) {
        // Superseded invocations must not publish a stale error over the newer state either.
        if (gen !== inferenceGenRef.current) return;
        reasonedStoreRef.current = null;
        reasonedKeyRef.current = null;
        entailedCacheRef.current = null;
        setInferenceStatus({
          kind: "error",
          message: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [buildReasonedStore],
  );

  // [OPUS-4.8] sq-tp1m — the PUBLIC mode setter is a plain state write; the effect below does the
  // (async) reasoning work. Keeping the two apart means import/update need only bump the store
  // epoch to trigger a closure rebuild, and the workspace-restore bridge just calls this setter.
  const setInferenceMode = React.useCallback((mode: WorkspaceInferenceMode) => {
    setInferenceModeState(mode);
  }, []);

  // [FABLE-5] sq-ixc3.14 — push the active workspace's federation egress allowlist. A plain
  // ref write: the next SERVICE-bearing run() hands exactly this list to the native engine's
  // strict fail-closed policy. Called by FederationBridge + the workspace lifecycle (switch /
  // restore), mirroring setInferenceMode's lockstep discipline.
  const setServiceAllowlist = React.useCallback((hosts: string[]) => {
    serviceAllowRef.current = hosts;
  }, []);

  // [FABLE-5] sq-ixc3.20 — the entailed-key set of the ACTIVE materialised closure, or null.
  // The key guard makes staleness impossible: the set is only handed out while it describes
  // exactly the closure `run()` queries against (same `mode:epoch[:rulesHash]` key).
  const entailedTripleKeys = React.useCallback((): ReadonlySet<string> | null => {
    const cached = entailedCacheRef.current;
    if (!cached || cached.key !== reasonedKeyRef.current) return null;
    return cached.keys;
  }, []);

  // [GPT-5.6] Displayable/provable facts for the ACTIVE closure, with the identical stale-key
  // guard as entailedTripleKeys. The cached array is immutable by convention to consumers.
  const entailedTripleFacts = React.useCallback((): readonly InferredFact[] | null => {
    const cached = entailedCacheRef.current;
    if (!cached || cached.key !== reasonedKeyRef.current) return null;
    return cached.facts;
  }, []);

  // [FABLE-5] sq-ixc3.20 — one witness derivation of a clicked triple under the active
  // regime, as proof-tree JSON. Runs over the ASSERTED base (the reasoner re-derives — the
  // proof must bottom out in asserted facts, so the closure is never the input). The wasm
  // reasoner is a stateless one-shot, so a click pays one materialisation-with-tracing; the
  // GUI only offers the affordance on entailed facts, so this is on-demand, never per-row.
  const explainWhy = React.useCallback(
    async (s: string, p: string, o: string): Promise<string> => {
      const base = storeRef.current;
      if (!base) {
        throw new Error("The engine is not ready yet — wait for the store to warm.");
      }
      const mode = inferenceMode;
      if (mode === "off") {
        throw new Error("Inference is off — there is no entailment regime to explain under.");
      }
      const reasoner = await loadReasoner();
      // Feature-detect the explain surface (the published bundle has it; an older synced
      // bundle degrades to this honest message rather than a `not a function` crash).
      const missing = (name: string) =>
        new Error(
          `This reason bundle lacks the ${name} proof surface — rebuild it with the ` +
            `explain feature (js: npm run build:reason-wasm).`,
        );
      if (mode === "n3") {
        const whyN3 = (reasoner as Partial<WasmReasoner>).whyN3;
        if (typeof whyN3 !== "function") throw missing("whyN3()");
        const rulesText = n3RulesRef.current
          .filter((d) => d.enabled !== false)
          .map((d) => d.text)
          .join("\n");
        // The SAME combined rules + folded-base document the closure build feeds reasonN3.
        return whyN3(rulesText + "\n" + storeToNTriples(base), s, p, o);
      }
      const why = (reasoner as Partial<WasmReasoner>).why;
      if (typeof why !== "function") throw missing("why()");
      return why(storeToNQuads(base), "nquads", modeToProfile(mode), s, p, o);
    },
    [inferenceMode],
  );

  // [sq-glo5r] — push updated N3 rules to the engine; bumps rulesEpoch so the
  // applyInferenceMode effect fires and rebuilds the N3 closure when rules change.
  const setN3Rules = React.useCallback((docs: WorkspaceRulesDoc[]) => {
    n3RulesRef.current = docs;
    rulesEpochRef.current += 1;
    setRulesEpoch(rulesEpochRef.current);
  }, []);

  // [OPUS-4.8] sq-tp1m — whenever the mode OR the base store content OR the N3 rules change,
  // reconcile the closure + status. The SINGLE place reasoning is applied. `rulesEpoch` is in
  // deps so a rules change (via setN3Rules) re-fires this effect when N3 is active.
  React.useEffect(() => {
    void applyInferenceMode(inferenceMode);
  }, [inferenceMode, storeEpoch, rulesEpoch, applyInferenceMode]);

  // [OPUS-4.8] sq-ixc3.13 — the Import drawer's ingest. A `file` import (a disk path) goes
  // through the NATIVE loader IPC (`load_path`: compressed + native-only HDT, no wasm-tab
  // ceiling) inside the Tauri shell; `paste`/`url` documents go through native `load_text` there
  // too. Either way the loader hands back the document as N-QUADS (named-graph-preserving), which
  // we MERGE (concatenate with the live store's N-Quads) or REPLACE, then re-load with
  // `loadDataset(_, "nquads")` so every named graph of both sides survives. On the hosted web
  // target (no native loader), `paste`/`url` parse directly in the in-tab WASM engine; a `file`
  // import is rejected there with a clear message (browsers cannot read an arbitrary disk path).
  // The actual ingest body (separated so `importRdf` can bracket it with the ingest signal).
  const runImport = React.useCallback(
    async (req: ImportRequest, Store: WasmStoreCtor): Promise<ImportResult> => {
      // 1. Decode the incoming document to N-Quads (native loader when available, else in-tab).
      let incomingNQuads: string;
      let added: number;
      let format = req.format;
      let loadedNatively = false;
      let bytes = req.text ? new Blob([req.text]).size : 0;

      const native = hasNativeLoader();
      if (req.kind === "file") {
        if (!native || !req.path) {
          throw new Error(
            "Importing a file from disk needs the desktop app (the native loader). On the web " +
              "target, paste the document or load it by URL instead.",
          );
        }
        const doc = await nativeLoadPath(req.path, req.format, req.preserveGraphs);
        if (!doc) throw new Error("The native loader is unavailable.");
        incomingNQuads = doc.nquads;
        added = doc.count;
        format = doc.format;
        loadedNatively = true;
        bytes = new Blob([incomingNQuads]).size;
      } else {
        // paste / url — a document body in `req.text`.
        const text = req.text ?? "";
        const doc: LoadedDocument | null = native
          ? await nativeLoadText(text, req.format, req.preserveGraphs)
          : null;
        if (doc) {
          incomingNQuads = doc.nquads;
          added = doc.count;
          format = doc.format;
          loadedNatively = true;
        } else {
          // In-tab WASM parse: build a store from the incoming doc, then serialise to N-Quads.
          const incomingStore = req.preserveGraphs
            ? Store.loadDataset(text, req.format)
            : Store.load(text, req.format);
          incomingNQuads = storeToNQuads(incomingStore);
          added = incomingNQuads === "" ? 0 : incomingNQuads.split("\n").length;
        }
      }

      // 2. Merge (add) or replace, then re-load the whole dataset as N-Quads.
      let combined = incomingNQuads;
      if (req.mode === "add" && storeRef.current) {
        const current = storeToNQuads(storeRef.current);
        combined = current === "" ? incomingNQuads : `${current}\n${incomingNQuads}`;
      }
      const merged = Store.loadDataset(combined, "nquads");
      storeRef.current = merged;
      // [OPUS-4.8] sq-tp1m — the base store changed: bump the epoch so an active regime rebuilds
      // its closure over the newly-imported data.
      storeEpochRef.current += 1;
      setStoreEpoch(storeEpochRef.current);

      // 3. Refresh the datasets tree + store size + on-device footprint from the new store.
      const { size, graphs: gs } = summariseGraphs(merged);
      setStoreSize(size);
      setStoreBytes(snapshotBytes(merged));
      setGraphs(gs);

      return { added, storeSize: size, loadedNatively, format, bytes };
    },
    [],
  );

  // [OPUS-4.8] sq-ixc3.13 — the Import drawer's ingest. A `file` import (a disk path) goes
  // through the NATIVE loader IPC (`load_path`: compressed + native-only HDT, no wasm-tab
  // ceiling) inside the Tauri shell; `paste`/`url` documents go through native `load_text` there
  // too. Either way the loader hands back the document as N-QUADS (named-graph-preserving), which
  // we MERGE (concatenate with the live store's N-Quads) or REPLACE, then re-load with
  // `loadDataset(_, "nquads")` so every named graph of both sides survives. On the hosted web
  // target (no native loader), `paste`/`url` parse directly in the in-tab WASM engine; a `file`
  // import is rejected there with a clear message (browsers cannot read an arbitrary disk path).
  const importRdf = React.useCallback(
    async (req: ImportRequest): Promise<ImportResult> => {
      const Store = ctorRef.current;
      if (!Store) {
        throw new Error("The engine is not ready yet — wait for the store to warm.");
      }
      // [OPUS-4.8] sq-vw3ax (#820) — mark the ingest in-flight so the status bar shows the
      // unintrusive ingest meter (the real file label + a live measured elapsed). Cleared in the
      // `finally` whether the import succeeds or fails. The loaders are synchronous, so this is an
      // honest "ingesting <file>…" indicator, not a fabricated progress percentage or ETA.
      setIngest({ label: req.label, startedAt: performance.now() });
      try {
        return await runImport(req, Store);
      } finally {
        setIngest(null);
      }
    },
    [runImport],
  );

  const run = React.useCallback(
    async (query: string, opts: RunOptions = {}): Promise<RunResult> => {
      const store = storeRef.current;
      if (!store) {
        const outcome: QueryOutcome = {
          kind: "error",
          message: "The engine is not ready yet — wait for the store to warm.",
        };
        return { outcome, latencyMs: 0 };
      }
      const mode = opts.mode ?? "run";
      const rowCap = opts.rowCap ?? DEFAULT_ROW_CAP;
      const signal = opts.signal;
      const form = classifyQuery(query);
      // [OPUS-4.8] sq-tp1m (#757) — pick the target store. When an inference regime is active,
      // READ queries run against the materialised forward-chaining CLOSURE so entailed triples
      // match; UPDATEs always mutate the ASSERTED base store (the closure is a derived, read-only
      // view — a successful UPDATE bumps the store epoch, invalidating the cached closure). A
      // missing / broken reasoner bundle is surfaced honestly, never silently queried un-reasoned.
      // The closure build is OUTSIDE the latency window below so `latencyMs` measures the query,
      // not the one-time materialisation (which is cached across queries).
      let target = store;
      if (inferenceMode !== "off" && form !== "update") {
        try {
          target = await buildReasonedStore(inferenceMode);
        } catch (err) {
          const message = err instanceof Error ? err.message : String(err);
          return {
            outcome: {
              kind: "error",
              message:
                `Inference is set to ${inferenceMode.toUpperCase()}, but the reasoning bundle ` +
                `could not run: ${message} Turn inference Off to query the asserted data, or ` +
                `rebuild the reasoner bundle (js: npm run build:reason-wasm).`,
            },
            latencyMs: 0,
          };
        }
      }
      const t0 = performance.now();
      let outcome: QueryOutcome;
      try {
        if (mode === "run" && (form === "select" || form === "ask") && queryUsesService(query)) {
          // [FABLE-5] sq-ixc3.14 — FEDERATED path. The in-tab WASM engine cannot evaluate a
          // SERVICE clause (no blocking cross-origin HTTP in a browser), so a SERVICE-bearing
          // SELECT/ASK routes to the desktop shell's NATIVE engine over IPC: a whole-dataset
          // N-Quads snapshot of the (possibly reasoned) target store + the query + the
          // per-workspace egress allowlist, evaluated under the engine's STRICT fail-closed
          // policy (an endpoint off the list is refused pre-HTTP — a typed error, never a
          // hang). On the web build this degrades HONESTLY with the native-only message
          // instead of running. The native call is not cooperatively cancellable; a Stop
          // during flight surfaces as "cancelled" once the (transport-bounded) call returns.
          if (!hasNativeFederation()) {
            outcome = { kind: "error", message: WEB_FEDERATION_MESSAGE };
          } else {
            const dataset = storeToNQuads(target);
            const json = await nativeServiceQuery(dataset, query, serviceAllowRef.current);
            if (signal?.aborted) {
              outcome = { kind: "cancelled" };
            } else if (json === null) {
              outcome = { kind: "error", message: WEB_FEDERATION_MESSAGE };
            } else {
              outcome = parseServiceResults(json, rowCap);
            }
          }
        } else if (mode === "explain" || mode === "analyze") {
          // EXPLAIN renders the planner's chosen plan; EXPLAIN ANALYZE also EXECUTES it and
          // traces the per-operator work (the wasm `explain` / `explainAnalyze` bindings, which
          // mirror `sparq_engine::explain[_analyze]` and the server's `explain=plan|analyze`).
          //
          // [FABLE-5] sq-ixc3.19 — three-way source selection for the plan explorer:
          //   * `native` (desktop only): the `explain_native` Tauri command over an N-Quads
          //     snapshot of the SAME target store (federation's snapshot semantics) — the one
          //     source with REAL per-operator wall nanos.
          //   * structured wasm (`explainPlanJson` / `explainPlanAnalyzeJson`, present in the
          //     published bundle): the typed tree; exact rows + q-error, nanos read 0 on wasm32.
          //     For ANALYZE exactly ONE binding runs — the text and JSON analyze forms would
          //     each execute the query, so calling both would double the measured work.
          //   * text (a lean bundle without the explain-json feature): today's `<pre>` plan.
          if (opts?.native) {
            const json = await nativeExplain(
              storeToNQuads(target),
              query,
              mode === "analyze",
            );
            if (signal?.aborted) {
              outcome = { kind: "cancelled" };
            } else if (json === null) {
              outcome = {
                kind: "error",
                message:
                  "Native EXPLAIN runs only in the desktop app — the hosted web build has no native engine. The in-tab EXPLAIN/ANALYZE still works (row counts and q-error are exact; wall times are unmeasured).",
              };
            } else {
              outcome = { kind: "explain", mode, plan: "", tree: parsePlanJson(json), source: "native" };
            }
          } else {
            const planCapable = target as WasmStore & Partial<PlanCapableStore>;
            if (
              typeof planCapable.explainPlanJson === "function" &&
              typeof planCapable.explainPlanAnalyzeJson === "function"
            ) {
              const json =
                mode === "analyze"
                  ? planCapable.explainPlanAnalyzeJson(query)
                  : planCapable.explainPlanJson(query);
              outcome = { kind: "explain", mode, plan: "", tree: parsePlanJson(json), source: "wasm" };
            } else {
              const plan = mode === "analyze" ? target.explainAnalyze(query) : target.explain(query);
              outcome = { kind: "explain", mode, plan, source: "wasm" };
            }
          }
        } else if (form === "ask") {
          const json = target.query(query);
          const parsed = JSON.parse(json) as SparqlResults;
          outcome = isAskResult(parsed)
            ? { kind: "ask", value: askValue(parsed) ?? false, rawJson: formatSparqlJson(parsed) }
            : { kind: "error", message: "ASK query did not return a boolean result." };
        } else if (form === "construct" || form === "describe") {
          const ntriples = target.queryQuads(query);
          const tripleCount = ntriples
            .split("\n")
            .filter((l) => l.trim().length > 0).length;
          outcome = { kind: "graph", ntriples, tripleCount };
        } else if (form === "update") {
          store.updateInPlace(query);
          // [OPUS-4.8] sq-tp1m — the asserted store changed: bump the epoch so an active regime
          // rebuilds its closure over the post-update data.
          storeEpochRef.current += 1;
          setStoreEpoch(storeEpochRef.current);
          // `size` reports the default graph only; recompute the full per-graph total below.
          const { size } = summariseGraphs(store);
          outcome = { kind: "update", sizeAfter: size };
        } else {
          // SELECT — STREAM the rows one batch at a time through the wasm cursor so a large
          // result never materialises whole in JS. We keep at most `rowCap` rows for the views;
          // rows beyond the cap are counted but dropped (the outcome records `truncated`). The
          // cooperative `signal` is checked between batches so Stop actually halts the pull.
          const kept: SparqlBinding[] = [];
          let total = 0;
          let cancelled = false;
          const meta = streamQueryRows(target, query, STREAM_BATCH_SIZE, (batch) => {
            if (signal?.aborted) {
              cancelled = true;
              // Throw to break streamQueryRows' loop; the cursor is freed in its `finally`.
              throw new AbortError();
            }
            total += batch.rows.length;
            for (const row of batch.rows) {
              if (kept.length < rowCap) kept.push(row);
            }
          });
          if (cancelled) {
            outcome = { kind: "cancelled" };
          } else {
            const parsed: SparqlResults = {
              head: { vars: meta.vars },
              results: { bindings: kept },
            };
            // `meta.rowCount` is the cursor's own total; prefer it (it covers an empty result's
            // single empty batch correctly), falling back to the counted total.
            const totalRows = meta.rowCount || total;
            outcome = {
              kind: "select",
              results: parsed,
              rawJson: formatSparqlJson(parsed),
              rowCount: kept.length,
              totalRows,
              truncated: totalRows > kept.length,
            };
          }
        }
      } catch (err) {
        // [FABLE-5] sq-ixc3.14 — a native SERVICE egress REFUSAL (the engine's stable marker)
        // is surfaced as the actionable fail-closed message (pointing at the per-workspace
        // allowlist); every other error passes through unchanged. A Tauri command rejection is
        // a plain string, so String(err) covers both shapes.
        const raw = err instanceof Error ? err.message : String(err);
        outcome =
          err instanceof AbortError || signal?.aborted
            ? { kind: "cancelled" }
            : { kind: "error", message: describeServiceRefusal(raw) ?? raw };
      }
      const latencyMs = performance.now() - t0;
      setLastLatencyMs(latencyMs);
      setLastRowCount(
        outcome.kind === "select"
          ? outcome.totalRows
          : outcome.kind === "graph"
            ? outcome.tripleCount
            : null,
      );
      // An UPDATE mutated the store; refresh the datasets tree.
      if (outcome.kind === "update") refreshSummary();
      return { outcome, latencyMs };
    },
    [refreshSummary, inferenceMode, buildReasonedStore],
  );

  // [OPUS-4.8] sq-ixc3.10/.12 — EXPLAIN / EXPLAIN ANALYZE is NOT a separate context method: it is
  // run(query, { mode: "explain" | "analyze" }) above, which surfaces an { kind: "explain" }
  // outcome through the SAME RunResult + measured-latency pipeline every other run uses. The Cmd-K
  // "Run EXPLAIN" verb and the workbench EXPLAIN/ANALYZE buttons both drive that single path, so
  // there is one EXPLAIN contract (this consolidates the standalone explain() #1018 had shipped).

  // [OPUS-4.8] sq-ixc3.11 — TriG dump of the live store, the input an operational tool (SHACL
  // validate-the-active-store) consumes. TriG (the serialise binding does NOT accept
  // "ntriples" — only turtle/trig/jsonld) preserves the default graph AND every named graph,
  // and is unabbreviated (`abbreviate=false`) so no caller-supplied prefix map can disagree.
  // `null` until the store warms or if a lean bundle lacks the binding.
  const serializeStore = React.useCallback((): string | null => {
    const store = storeRef.current;
    if (!store) return null;
    // The GUI bundle is built with `serialize-rdf`, but the runtime-loaded bundle decides
    // whether the binding exists; keep a defensive view so a lean bundle yields a clear empty
    // result rather than a `serialize is not a function` crash.
    const serialize = (store as { serialize?: WasmStore["serialize"] }).serialize;
    if (typeof serialize !== "function") return null;
    return store.serialize("trig", false, null, false, null);
  }, []);

  // [OPUS-4.8] sq-xvj9 — the USER-facing dataset export the DatasetViewer downloads. Serialises the
  // whole live store PRETTY (indented + sorted) and ABBREVIATED against the site's COMMON_PREFIXES
  // (sq-l5kr's caller-supplied prefix map), through the SAME engine writer as `serializeStore`.
  // `turtle` carries the default graph only (Turtle has no named-graph syntax); `trig`/`jsonld`
  // carry the whole dataset. `null` if the store is cold or a lean bundle lacks the binding.
  const exportStore = React.useCallback((format: ExportFormat): string | null => {
    const store = storeRef.current;
    if (!store) return null;
    const serialize = (store as { serialize?: WasmStore["serialize"] }).serialize;
    if (typeof serialize !== "function") return null;
    return store.serialize(EXPORT_SERIALIZE_FORMAT[format], true, null, true, EXPORT_PREFIXES);
  }, []);

  // [OPUS-4.8] sq-ixc3.13 — the N-Quads whole-dataset snapshot the workspace persists (sq-atb0).
  const snapshotStore = React.useCallback((): string | null => {
    const store = storeRef.current;
    if (!store) return null;
    return storeToNQuads(store);
  }, []);

  // [GPT-5.6] sq-3eukz — picker resources come directly from the current store so every store
  // epoch (including workspace hydration) can synchronously replace the candidate list.
  const listFormResources = React.useCallback((): FormResource[] => {
    const store = storeRef.current;
    if (!store) return [];
    const parsed = JSON.parse(store.query(FORM_RESOURCES_QUERY)) as SparqlResults;
    const resources: FormResource[] = [];
    for (const binding of parsed.results?.bindings ?? []) {
      const term = binding["resource"];
      if (term?.type === "uri") resources.push({ kind: "iri", value: term.value });
      if (term?.type === "bnode") resources.push({ kind: "bnode", value: term.value });
    }
    return resources;
  }, []);

  // [GPT-5.6] sq-3eukz — structural feature detection keeps the GUI independently buildable
  // until the opt-in wasm forms binding lands. Preserve the receiver when invoking wasm-bindgen.
  const wasmDeriveForm = React.useCallback(
    (
      data: string,
      shapes: string,
      focus: string,
      format: string,
      optionsJson: string,
    ): string | null => {
      const store = storeRef.current;
      if (!store) return null;
      const derive = (
        store as unknown as {
          deriveForm?: (
            data: string,
            shapes: string,
            focus: string,
            format: string,
            optionsJson: string,
          ) => string;
        }
      ).deriveForm;
      if (typeof derive !== "function") return null;
      return derive.call(store, data, shapes, focus, format, optionsJson);
    },
    [],
  );

  // [OPUS-4.8] sq-ixc3.13 — whether the native loader IPC is reachable (inside the Tauri shell).
  // Computed once on mount (the runtime never changes mid-session) so the status bar / drawer can
  // label the loader honestly without re-detecting on every render.
  const [nativeLoaderAvailable, setNativeLoaderAvailable] = React.useState(false);
  React.useEffect(() => {
    setNativeLoaderAvailable(hasNativeLoader());
  }, []);

  // [FABLE-5] sq-ixc3.14 — whether the native federated-query IPC is reachable (inside the
  // Tauri shell). Same once-on-mount discipline as nativeLoaderAvailable, so the Query tool
  // can label the SERVICE run location honestly without re-detecting per render.
  const [nativeFederationAvailable, setNativeFederationAvailable] = React.useState(false);
  React.useEffect(() => {
    setNativeFederationAvailable(hasNativeFederation());
  }, []);

  const value = React.useMemo<EngineContextValue>(
    () => ({
      status,
      storeSize,
      storeBytes,
      diskBytes,
      refreshDiskUsage,
      ingest,
      graphs,
      lastLatencyMs,
      lastRowCount,
      run,
      serializeStore,
      exportStore,
      importRdf,
      nativeLoaderAvailable,
      listFormResources,
      wasmDeriveForm,
      snapshotStore,
      storeEpoch,
      hydrateFromSnapshot,
      inferenceMode,
      inferenceStatus,
      setInferenceMode,
      entailedTripleKeys,
      entailedTripleFacts,
      explainWhy,
      setN3Rules,
      setServiceAllowlist,
      nativeFederationAvailable,
    }),
    [
      status,
      storeSize,
      storeBytes,
      diskBytes,
      refreshDiskUsage,
      ingest,
      graphs,
      lastLatencyMs,
      lastRowCount,
      run,
      serializeStore,
      exportStore,
      importRdf,
      nativeLoaderAvailable,
      listFormResources,
      wasmDeriveForm,
      snapshotStore,
      storeEpoch,
      hydrateFromSnapshot,
      inferenceMode,
      inferenceStatus,
      setInferenceMode,
      entailedTripleKeys,
      entailedTripleFacts,
      explainWhy,
      setN3Rules,
      setServiceAllowlist,
      nativeFederationAvailable,
    ],
  );

  return <EngineContext.Provider value={value}>{children}</EngineContext.Provider>;
}

export function useEngine(): EngineContextValue {
  const ctx = React.useContext(EngineContext);
  if (!ctx) throw new Error("useEngine must be used within an <EngineProvider>");
  return ctx;
}
